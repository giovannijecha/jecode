//! The process sheet, footer and completion rows painted through the actual
//! Windows ConPTY and terminal renderer, narrow and wide, in both palettes.
use super::{
    Terminal,
    native_mcp::{paint, wait_for_width},
};
use crate::{
    context::Status,
    tools::processes::{Completion, Page, Process},
    tui::{
        Feedback, Kind,
        activity::Activity,
        feedback::Ending,
        processes::{Logs, Panel},
        state::State,
    },
};
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires Windows ConPTY and PowerShell for native process panels"]
fn native_windows_process_panels() {
    let directory = crate::test_support::Directory::new();
    let executable = std::env::current_exe().unwrap();
    let result = super::native_script::pty(
        super::native_pty_source(),
        "Run",
        &[
            &executable.to_string_lossy(),
            &directory.path().to_string_lossy(),
            "process-panels",
        ],
        &directory,
        Duration::from_secs(60),
        false,
    );
    assert_eq!(
        result.exit_code,
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&result.stderr),
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!result.timed_out);
}

pub(super) fn console() {
    let terminal = Terminal::open().unwrap();
    for columns in [44, 80, 100] {
        wait_for_width(&terminal, columns);
        for light in [false, true] {
            crate::tui::theme::set_light(light);
            paint(columns, sheet(columns, list()), "b background");
            paint(columns, sheet(columns, list()), "exited 1");
            paint(columns, sheet(columns, output()), "following");
            // Narrower, the aside gives way to the breadcrumb, as every sheet's does.
            if columns >= 80 {
                paint(columns, sheet(columns, long_output()), "exited 1");
                paint(columns, sheet(columns, long_output()), "2m 14s");
            }
            paint(columns, footer(columns), "PROCESS");
            paint(columns, footer(columns), "2 running");
            paint(columns, api_context(columns), "27%");
            paint(
                columns,
                combined_footer(columns),
                if columns == 44 {
                    "2 proc"
                } else {
                    "2 processes"
                },
            );
            paint(columns, finished(columns, "exited", Some(1)), "Exit 1");
            paint(columns, finished(columns, "exited", None), "Exit unknown");
            paint(columns, finished(columns, "exited", Some(0)), "Done");
            paint(columns, finished(columns, "stopped", None), "stopped");
            paint(columns, finished(columns, "timed_out", None), "Timed out");
            paint(columns, completion_status(columns, true), "Thinking");
            paint(columns, completion_status(columns, false), "Done");
            paint(columns, completion_status(columns, false), "4 tools");
            if columns >= 80 {
                paint(columns, completion_status(columns, false), "29.4k");
                paint(columns, completion_status(columns, false), "2.1k");
            }
        }
        crate::tui::theme::set_light(false);
        println!("PROCESS_NATIVE_{columns}_VERIFIED");
        io::stdout().flush().unwrap();
    }
    drop(terminal);
}

fn process(id: &str, command: &str, status: &'static str, code: Option<i32>) -> Process {
    let started = Instant::now() - Duration::from_secs(134);
    Process {
        id: id.into(),
        command: command.into(),
        status,
        exit_code: code,
        error: None,
        started,
        finished: (status != "running").then_some(started + Duration::from_secs(9)),
        awaited: false,
    }
}

fn list() -> Panel {
    let mut running = process("process-1", "cargo test --locked", "running", None);
    running.awaited = true;
    Panel::new(vec![
        running,
        process("process-2", "npm run dev", "running", None),
        process("process-3", "cargo build --locked", "exited", Some(0)),
        process("process-4", "npm run lint", "exited", Some(1)),
    ])
}

fn output() -> Panel {
    let process = process("process-2", "npm run dev", "running", None);
    let text = "ready on :5173\n";
    let mut logs = Logs {
        id: process.id.clone(),
        process: Some(process),
        stream: 0,
        views: Default::default(),
        interactive: false,
        input_mode: false,
    };
    logs.views[0].pages.push_back(Page {
        text: text.into(),
        start: 0,
        end: text.len() as u64,
        length: text.len() as u64,
    });
    let mut panel = list();
    panel.logs = Some(logs);
    panel
}

/// A long command with a real process ID, which must not cost it its outcome.
fn long_output() -> Panel {
    let mut process = process(
        "process-1793119373364-13804-6",
        &format!("cargo test {}", "--long-flag ".repeat(20)),
        "exited",
        Some(1),
    );
    process.finished = Some(Instant::now());
    let mut panel = list();
    panel.logs = Some(Logs {
        id: process.id.clone(),
        process: Some(process),
        stream: 0,
        views: Default::default(),
        interactive: false,
        input_mode: false,
    });
    panel
}

fn sheet(columns: usize, panel: Panel) -> State {
    State {
        width: columns,
        height: 30,
        processes: Some(panel),
        ..State::default()
    }
}

fn footer(columns: usize) -> State {
    State {
        width: columns,
        height: 30,
        background_processes: 2,
        context: Some(Status {
            tokens: Some(61_000),
            capacity: Some(100_000),
            auto_compact: 80,
            pending: None,
        }),
        ..State::default()
    }
}

fn api_context(columns: usize) -> State {
    State {
        width: columns,
        height: 30,
        context: Some(Status {
            tokens: Some(286_600),
            capacity: Some(1_050_000),
            auto_compact: 80,
            pending: None,
        }),
        ..State::default()
    }
}

fn combined_footer(columns: usize) -> State {
    let mut state = footer(columns);
    state.collaboration = vec![crate::json::Value::object([(
        "state",
        crate::json::Value::string("running"),
    )])];
    state
}

fn finished(columns: usize, status: &'static str, exit_code: Option<i32>) -> State {
    let mut state = State {
        width: columns,
        height: 30,
        ..State::default()
    };
    state.message(Kind::User, "Build it.");
    state.process_completion(
        Completion {
            sequence: 1,
            id: "process-1".into(),
            command: "cargo test --locked".into(),
            status,
            exit_code,
            error: None,
            elapsed: Duration::from_secs(134),
            stdout_file: "output:s:1:stdout".into(),
            stderr_file: "output:s:1:stderr".into(),
            tails: [String::new(), String::new()],
            tty: false,
            stop_requested: false,
        }
        .value(),
    );
    state
}

fn completion_status(columns: usize, working: bool) -> State {
    let mut state = finished(columns, "exited", Some(101));
    if working {
        let mut activity = Activity::with_elapsed(Duration::from_millis(38_200));
        activity.sent = 29_400;
        activity.generated = 2_100;
        state.activity = Some(activity);
        state.status = "Thinking".into();
    } else {
        state.notify(Feedback::ending(
            Ending::Done,
            "38.2s · 4 tools · ↑ 29.4k · ↓ 2.1k",
        ));
    }
    state
}
