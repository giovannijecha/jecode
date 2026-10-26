//! Tail frames from the real Rust view, paired with Display regression tests.
use crate::{
    events::{Event, tool_summary},
    json::Value,
    tui::{
        state::{Item, Kind, State},
        view,
    },
};
use std::time::Duration;

fn begin(state: &mut State, id: &str, name: &str, key: &str, target: &str) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments: Value::object([
            (key, Value::string(target)),
            ("work", Value::string("Inspect sources")),
        ]),
    });
}
fn done(state: &mut State, id: &str, name: &str, result: Value) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: tool_summary(name, &result),
        result,
    });
}
fn observe(state: &mut State, phase: &str) {
    for item in &mut state.items {
        if let Item::Tool {
            name, presentation, ..
        } = item
        {
            presentation.started = None;
            presentation.elapsed = Some(Duration::from_millis(if name == "bash" {
                2400
            } else {
                400
            }));
        }
    }
    println!("LIVE {} {phase}", state.width);
    for line in view::frame(state, "fixture/model", "Jecode").live {
        println!("{}", line.paint(state.width));
    }
}

#[test]
#[ignore = "prints chronological native tail frames at 80 and 40 cells"]
fn chapter_live() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            height: 20,
            ..State::default()
        };
        state.message(
            Kind::Assistant,
            &(1..=25)
                .map(|n| format!("Earlier transcript line {n:02}.  \n"))
                .collect::<String>(),
        );
        state.begin_tool_turn();
        begin(&mut state, "b1", "bash", "command", "printf one");
        observe(&mut state, "01 Bash started");
        done(
            &mut state,
            "b1",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        observe(&mut state, "02 Bash finished");
        begin(&mut state, "r1", "read", "path", "src/a.rs");
        observe(&mut state, "03 Chapter formed");
        done(
            &mut state,
            "r1",
            "read",
            Value::object([("lines_returned", Value::number(9))]),
        );
        observe(&mut state, "04 Read finished");
        begin(&mut state, "r2", "read", "path", "missing.rs");
        observe(&mut state, "05 Read started");
        done(
            &mut state,
            "r2",
            "read",
            Value::object([("error", Value::string("permission denied"))]),
        );
        observe(&mut state, "06 Read failed");
        begin(&mut state, "b2", "bash", "command", "printf two");
        observe(&mut state, "07 Bash started");
        done(
            &mut state,
            "b2",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        observe(&mut state, "08 Bash finished");
        begin(&mut state, "browser", "browser", "action", "snapshot");
        observe(&mut state, "09 Browser started");
        done(
            &mut state,
            "browser",
            "browser",
            Value::object([("snapshot_path", Value::string("tmp:browser/result.txt"))]),
        );
        observe(&mut state, "10 Browser finished");
        state.close_tools();
        state.finish_tool_turn();
        observe(&mut state, "11 Turn closed");
    }
}
