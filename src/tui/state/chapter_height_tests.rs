use super::*;
use crate::tui::line::Line;

mod singleton;

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
fn failed_command_and_read_do_not_repeat_chapter_facts_or_lose_height() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        let mut previous = [0; 2];
        let mut check = |state: &State, phase: &str, members: usize| {
            for (slot, keycap) in [false, true].into_iter().enumerate() {
                let current = height(state, keycap);
                assert!(
                    current >= previous[slot],
                    "{columns} columns, {phase}, keycap {keycap}: {current} < {}\n{}",
                    previous[slot],
                    shown(state)
                );
                previous[slot] = current;
            }
            if members >= 2 {
                let lines: Vec<_> = state.rows()[..members]
                    .iter()
                    .flat_map(|rows| rows.iter())
                    .map(Line::plain)
                    .collect();
                let text = lines.join("\n");
                assert!(
                    lines.iter().all(|line| !line.trim().is_empty()),
                    "{columns} columns, {phase}: {text}"
                );
                assert_eq!(
                    text.to_ascii_lowercase().matches("exit 101").count(),
                    1,
                    "{columns} columns, {phase}: {text}"
                );
                assert!(
                    text.matches("Read  missing.rs").count() <= 1,
                    "{columns} columns, {phase}: {text}"
                );
            }
        };
        state.begin_tool_turn();
        start(
            &mut state,
            "command",
            "bash",
            Value::object([
                ("command", Value::string("printf long_argument_".repeat(16))),
                ("description", Value::string("Run parser checks")),
                ("work", Value::string("Inspect")),
            ]),
        );
        check(&state, "command starts", 1);
        finish(
            &mut state,
            "command",
            "bash",
            Value::object([
                ("exit_code", Value::number(101)),
                (
                    "stderr",
                    Value::string("error one\nerror two\nerror three\n"),
                ),
            ]),
        );
        check(&state, "command fails", 1);
        start(
            &mut state,
            "read",
            "read",
            Value::object([
                ("path", Value::string("missing.rs")),
                ("work", Value::string("Inspect")),
            ]),
        );
        check(&state, "chapter forms", 2);
        finish(
            &mut state,
            "read",
            "read",
            Value::object([("error", Value::string("permission denied"))]),
        );
        check(&state, "read fails", 2);
        start(
            &mut state,
            "next",
            "bash",
            Value::object([
                ("command", Value::string("printf next")),
                ("work", Value::string("Inspect")),
            ]),
        );
        check(&state, "next call starts", 3);
        finish(
            &mut state,
            "next",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        check(&state, "next call finishes", 3);
        state.close_tools();
        state.finish_tool_turn();
        check(&state, "turn closes", 3);
        let chapter_height = height(&state, true);
        state.begin_tool_turn();
        start(
            &mut state,
            "new-turn",
            "read",
            Value::object([("path", Value::string("src/new.rs"))]),
        );
        check(&state, "new turn takes keycap", 3);
        let mut renderer = rows::Rows::default();
        let old_height: usize = (0..3)
            .map(|index| {
                renderer
                    .render_at(&state.items, index, columns, false)
                    .len()
            })
            .sum();
        assert!(
            old_height >= chapter_height,
            "{columns} columns: old chapter {old_height} < {chapter_height}"
        );
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
fn first_ordinary_call_that_fails_keeps_error_detail_without_duplicate_activity() {
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
    assert_eq!(failed.matches("Read  missing.rs").count(), 1);
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
