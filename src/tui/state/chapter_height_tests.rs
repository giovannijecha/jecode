use super::*;
use crate::tui::line::Line;

fn start(state: &mut State, id: &str, name: &str, arguments: Value) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: name.into(),
        arguments,
    });
}

fn finish(state: &mut State, id: &str, name: &str, result: Value) {
    state.event(Event::ToolFinished {
        id: id.into(),
        name: name.into(),
        summary: crate::events::tool_summary(name, &result),
        result,
    });
}

fn height(state: &State, keycap: bool) -> usize {
    let mut renderer = rows::Rows::default();
    (0..state.items.len())
        .map(|index| {
            renderer
                .render_at(&state.items, index, state.width, keycap && index == 0)
                .len()
        })
        .sum()
}

fn shown(state: &State) -> String {
    state
        .rows()
        .into_iter()
        .flatten()
        .map(|row| row.plain())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn chapter_keycap_handoff_keeps_content_height_without_blank_rows() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "first",
            "bash",
            Value::object([
                ("command", Value::string("printf first")),
                ("work", Value::string("Inspect")),
            ]),
        );
        finish(
            &mut state,
            "first",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        let singleton_height = height(&state, true);
        start(
            &mut state,
            "second",
            "bash",
            Value::object([
                ("command", Value::string("printf second")),
                ("work", Value::string("Inspect")),
            ]),
        );
        finish(
            &mut state,
            "second",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        let chapter_height = height(&state, true);
        assert!(chapter_height >= singleton_height);
        state.close_tools();
        state.finish_tool_turn();
        state.begin_tool_turn();
        start(
            &mut state,
            "next",
            "read",
            Value::object([("path", Value::string("src/next.rs"))]),
        );
        let mut renderer = rows::Rows::default();
        let old_chapter_height: usize = (0..2)
            .map(|index| {
                renderer
                    .render_at(&state.items, index, columns, false)
                    .len()
            })
            .sum();
        assert!(
            old_chapter_height >= chapter_height,
            "{columns} columns: {old_chapter_height} < {chapter_height}"
        );
        let chapter: Vec<_> = state.rows()[..2]
            .iter()
            .flat_map(|rows| rows.iter())
            .map(Line::plain)
            .collect();
        assert!(
            chapter.iter().all(|row| !row.trim().is_empty()),
            "{columns} columns: {chapter:?}"
        );
    }
}

#[test]
fn failed_command_singleton_keeps_height_without_raw_chapter_streams() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "failed",
            "bash",
            Value::object([
                (
                    "command",
                    Value::string("printf very_long_argument_".repeat(12)),
                ),
                ("description", Value::string("Run parser checks")),
                ("work", Value::string("Inspect")),
            ]),
        );
        finish(
            &mut state,
            "failed",
            "bash",
            Value::object([
                ("exit_code", Value::number(101)),
                (
                    "stderr",
                    Value::string("raw-error-one\nraw-error-two\nraw-error-three\n"),
                ),
            ]),
        );
        let singleton = height(&state, false);
        let singleton_with_keycap = height(&state, true);
        assert!(
            singleton >= 6,
            "{columns} columns: singleton {singleton}\n{}",
            shown(&state)
        );
        start(
            &mut state,
            "second",
            "bash",
            Value::object([
                ("command", Value::string("printf second")),
                ("work", Value::string("Inspect")),
            ]),
        );
        let chapter = shown(&state);
        assert!(
            height(&state, false) >= singleton,
            "{columns} columns: {} < {singleton}\n{chapter}",
            height(&state, false)
        );
        assert!(
            height(&state, true) >= singleton_with_keycap,
            "{columns} columns: {} < {singleton_with_keycap} with keycap\n{chapter}",
            height(&state, true)
        );
        assert!(!chapter.contains("raw-error-one"));
        assert!(!chapter.contains("raw-error-two"));
        assert!(!chapter.contains("raw-error-three"));
    }
}

#[test]
fn pending_browser_singleton_keeps_height_when_next_browser_starts() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "first",
            "browser",
            Value::object([
                ("action", Value::string("evaluate")),
                (
                    "expression",
                    Value::string("const value = browser.evaluate(); ".repeat(12)),
                ),
                ("work", Value::string("Inspect")),
            ]),
        );
        let singleton = height(&state, false);
        assert!(singleton >= 4, "{columns} columns: singleton {singleton}");
        start(
            &mut state,
            "second",
            "browser",
            Value::object([
                ("action", Value::string("snapshot")),
                ("work", Value::string("Inspect")),
            ]),
        );
        assert!(
            height(&state, false) >= singleton,
            "{columns} columns: {} < {singleton}\n{}",
            height(&state, false),
            shown(&state)
        );
        assert!(shown(&state).contains("Browser  snapshot"));
    }
}

#[test]
fn long_singletons_keep_their_height_when_the_second_call_forms_a_chapter() {
    let script = "const value = await browser.evaluate(veryLongExpression); ".repeat(8);
    let error = "Could not read a long path: permission denied. ".repeat(6);
    let notice = Value::object([
        ("exit_code", Value::number(0)),
        (
            "checkpoint",
            Value::object([
                ("status", Value::string("partial")),
                (
                    "reason",
                    Value::string("Several untracked paths could not be captured before execution"),
                ),
            ]),
        ),
        ("stdout_truncated", Value::Bool(true)),
        ("stdout_bytes", Value::number(2048)),
        ("output_limit_bytes", Value::number(1024)),
    ]);
    for columns in [80, 40] {
        for (name, arguments, result) in [
            (
                "browser",
                Value::object([
                    ("action", Value::string("evaluate")),
                    ("expression", Value::string(&script)),
                    ("work", Value::string("Inspect")),
                ]),
                Value::object([]),
            ),
            (
                "read",
                Value::object([
                    ("path", Value::string("src/very-long-name.rs")),
                    ("work", Value::string("Inspect")),
                ]),
                Value::object([("error", Value::string(&error))]),
            ),
            (
                "bash",
                Value::object([
                    ("command", Value::string("printf before")),
                    ("work", Value::string("Inspect")),
                ]),
                notice.clone(),
            ),
        ] {
            let mut state = State {
                width: columns,
                ..State::default()
            };
            state.begin_tool_turn();
            start(&mut state, "first", name, arguments);
            finish(&mut state, "first", name, result);
            let before = [height(&state, false), height(&state, true)];
            start(
                &mut state,
                "second",
                "browser",
                Value::object([
                    ("action", Value::string("snapshot")),
                    ("tab_id", Value::string("tab-1")),
                    ("work", Value::string("Inspect")),
                ]),
            );
            for (slot, keycap) in [false, true].into_iter().enumerate() {
                assert!(
                    height(&state, keycap) >= before[slot],
                    "{columns} columns, {name}, keycap {keycap}: {} < {}\n{}",
                    height(&state, keycap),
                    before[slot],
                    shown(&state)
                );
            }
        }
    }
}

#[test]
fn first_ordinary_call_that_fails_leaves_a_frozen_activity_receipt() {
    let mut state = State::default();
    state.begin_tool_turn();
    start(
        &mut state,
        "bash",
        "bash",
        Value::object([
            ("command", Value::string("printf before")),
            ("work", Value::string("Inspect")),
        ]),
    );
    finish(
        &mut state,
        "bash",
        "bash",
        Value::object([("exit_code", Value::number(0))]),
    );
    start(
        &mut state,
        "read",
        "read",
        Value::object([
            ("path", Value::string("missing.rs")),
            ("work", Value::string("Inspect")),
        ]),
    );
    let running = height(&state, false);
    assert!(shown(&state).contains("Read  missing.rs"));
    finish(
        &mut state,
        "read",
        "read",
        Value::object([("error", Value::string("permission denied"))]),
    );
    assert!(height(&state, false) >= running);
    let failed = shown(&state);
    assert!(failed.contains("Read  missing.rs"));
    assert!(failed.contains("failed"));
    assert!(failed.contains("permission denied"));
    state.close_tools();
    state.finish_tool_turn();
    assert_eq!(shown(&state), failed);
}

#[test]
fn expanding_a_read_chapter_does_not_reserve_its_source_rows_as_blank_space() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        state.begin_tool_turn();
        for index in 0..2 {
            let id = format!("read-{index}");
            start(
                &mut state,
                &id,
                "read",
                Value::object([
                    ("path", Value::string(format!("src/{index}.rs"))),
                    ("work", Value::string("Inspect")),
                ]),
            );
            let content = (1..=24)
                .map(|line| format!("{line}: source {index} line {line}\n"))
                .collect::<String>();
            finish(
                &mut state,
                &id,
                "read",
                Value::object([
                    ("content", Value::string(content)),
                    ("lines_returned", Value::number(24)),
                    ("offset", Value::number(1)),
                ]),
            );
        }
        state.close_tools();
        state.finish_tool_turn();
        assert_eq!(state.toggle_tools(), Some((true, 2)));
        let rows = state.rows();
        let lines: Vec<_> = rows.into_iter().flatten().map(|row| row.plain()).collect();
        for index in 0..2 {
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains(&format!("source {index} line 24")))
            );
        }
        let mut run = 0;
        let mut longest = 0;
        for line in &lines {
            run = if line.trim().is_empty() { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        assert!(
            longest <= 2,
            "{columns} columns: {longest} blank rows\n{}",
            lines.join("\n")
        );
    }
}
