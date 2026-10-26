use super::*;
use crate::tui::{line::Line, theme::BAD};

fn args<const N: usize>(fields: [(&str, &str); N]) -> Value {
    Value::object(fields.map(|(key, value)| (key, Value::string(value))))
}

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

fn done(state: &mut State, id: &str, name: &str, arguments: Value, result: Value) {
    start(state, id, name, arguments);
    finish(state, id, name, result);
}

fn block(rows: &[Line]) -> String {
    rows.iter().map(Line::plain).collect::<Vec<_>>().join("\n")
}

fn heading(rows: &[Line]) -> &Line {
    rows.iter()
        .find(|row| row.plain().starts_with("● "))
        .expect("chapter heading")
}

#[test]
fn mixed_routine_calls_share_one_summary_and_expand_to_original_cards() {
    let mut state = State {
        width: 120,
        ..State::default()
    };
    state.begin_tool_turn();
    done(
        &mut state,
        "read",
        "read",
        args([("path", "src/main.rs"), ("work", "Inspect app")]),
        Value::object([
            ("content", Value::string("1: fn main() {}\n")),
            ("lines_returned", Value::number(1)),
            ("offset", Value::number(1)),
        ]),
    );
    done(
        &mut state,
        "bash",
        "bash",
        args([("command", "printf example"), ("work", "Inspect app")]),
        Value::object([
            ("stdout", Value::string("first\n\nlast\n")),
            ("exit_code", Value::number(0)),
        ]),
    );
    done(
        &mut state,
        "web",
        "web",
        args([("action", "fetch"), ("work", "Inspect app")]),
        Value::object([("status", Value::number(200))]),
    );
    done(
        &mut state,
        "browser",
        "browser",
        args([("action", "screenshot"), ("work", "Inspect app")]),
        args([("screenshot_path", "tmp:browser/shot.png")]),
    );
    done(
        &mut state,
        "process",
        "process",
        args([("action", "list"), ("work", "Inspect app")]),
        Value::object([("processes", Value::Array(vec![]))]),
    );
    state.close_tools();
    state.finish_tool_turn();

    let rows = state.rows();
    assert_eq!(rows.len(), 5);
    assert_eq!(heading(&rows[0]).plain(), "● Inspect app (5 calls)");
    let summary = block(&rows[0]);
    for fact in [
        "1 file read",
        "1 web call completed",
        "1 screenshot",
        "1 command completed",
        "1 process call completed",
    ] {
        assert!(summary.contains(fact), "missing {fact}: {summary}");
    }
    assert!(!summary.contains("What happened") && !summary.contains("Bash("));
    assert!(rows[1..].iter().all(Vec::is_empty));

    assert_eq!(state.toggle_tools(), Some((true, 5)));
    let opened = state.rows();
    for (index, target) in [
        "Read(src/main.rs)",
        "Bash(printf example)",
        "Web(fetch)",
        "Browser(screenshot)",
        "Process(list)",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(block(&opened[index]).contains(target), "{target}");
    }
    let bash = &opened[1];
    let first = bash
        .iter()
        .position(|row| row.plain() == "    first")
        .unwrap();
    assert_eq!(bash[first + 1].plain(), "    ");
    assert_eq!(bash[first + 2].plain(), "    last");
    assert_eq!(state.toggle_tools(), Some((false, 5)));
    assert!(state.rows()[1..].iter().all(Vec::is_empty));
}

#[test]
fn trimmed_labels_group_and_missing_or_blank_labels_use_activity() {
    let mut state = State::default();
    state.begin_tool_turn();
    for (index, work) in [" Audit ", "Audit", "", "   ", "Review"]
        .into_iter()
        .enumerate()
    {
        done(
            &mut state,
            &format!("r{index}"),
            "read",
            if index == 2 {
                args([("path", "src/a.rs")])
            } else {
                args([("path", "src/a.rs"), ("work", work)])
            },
            Value::object([("content", Value::string("1: a\n"))]),
        );
    }
    state.close_tools();
    let rows = state.rows();
    assert_eq!(heading(&rows[0]).plain(), "● Audit (2 calls)");
    assert!(rows[1].is_empty());
    assert_eq!(heading(&rows[2]).plain(), "● Activity (2 calls)");
    assert!(rows[3].is_empty());
    assert_eq!(heading(&rows[4]).plain(), "● Review (1 call)");
}

#[test]
fn narration_mutations_checkpoint_and_skill_reads_break_chapters() {
    let mut state = State::default();
    state.begin_tool_turn();
    let routine = || args([("path", "src/a.rs"), ("work", "Inspect")]);
    let read_result = || Value::object([("content", Value::string("1: a\n"))]);
    done(&mut state, "r0", "read", routine(), read_result());
    state.event(Event::Message {
        text: "A note.".into(),
    });
    done(&mut state, "r1", "read", routine(), read_result());
    for (index, (name, arguments)) in [
        ("write", args([("path", "src/a.rs")])),
        ("edit", args([("path", "src/a.rs")])),
        ("checkpoint", args([("action", "list")])),
    ]
    .into_iter()
    .enumerate()
    {
        done(
            &mut state,
            &format!("x{index}"),
            name,
            arguments,
            Value::object([]),
        );
        done(
            &mut state,
            &format!("r{}", index + 2),
            "read",
            routine(),
            read_result(),
        );
    }
    done(
        &mut state,
        "skill",
        "read",
        args([("path", "skill:review/SKILL.md"), ("work", "Inspect")]),
        read_result(),
    );
    done(&mut state, "last", "read", routine(), read_result());
    state.close_tools();
    let rows = state.rows();
    let headings: Vec<_> = rows
        .iter()
        .filter_map(|rows| rows.iter().find(|row| row.plain().starts_with("● ")))
        .map(Line::plain)
        .collect();
    assert_eq!(
        headings
            .iter()
            .filter(|text| text.starts_with("● Inspect"))
            .count(),
        6
    );
    for expected in ["Write(", "Update(", "Checkpoint(", "Skill("] {
        assert!(
            headings.iter().any(|text| text.contains(expected)),
            "{headings:?}"
        );
    }
    assert!(headings.iter().all(|text| !text.contains("(2 calls)")));
}

#[test]
fn adjacent_turns_never_merge_and_ctrl_o_opens_only_the_latest() {
    let mut state = State::default();
    for turn in ["old", "new"] {
        state.begin_tool_turn();
        for index in 0..2 {
            done(
                &mut state,
                &format!("{turn}-{index}"),
                "read",
                args([("path", &format!("{turn}-{index}.rs")), ("work", "Inspect")]),
                Value::object([("content", Value::string("1: kept\n"))]),
            );
        }
        state.close_tools();
        state.finish_tool_turn();
    }
    let rows = state.rows();
    assert_eq!(heading(&rows[0]).plain(), "● Inspect (2 calls)");
    assert!(rows[1].is_empty());
    assert_eq!(heading(&rows[2]).plain(), "● Inspect (2 calls)");
    assert!(rows[3].is_empty());
    assert_eq!(state.toggle_tools(), Some((true, 2)));
    let opened = state.rows();
    assert!(!block(&opened[0]).contains("Read(old-0.rs)"));
    assert!(!block(&opened[1]).contains("Read(old-1.rs)"));
    assert!(block(&opened[2]).contains("Read(new-0.rs)"));
    assert!(block(&opened[3]).contains("Read(new-1.rs)"));
}

#[test]
fn a_pending_call_reopens_its_chapter_header_until_results_arrive() {
    let mut state = State::default();
    state.begin_tool_turn();
    for index in 0..2 {
        done(
            &mut state,
            &format!("done-{index}"),
            "bash",
            args([("command", "printf ok"), ("work", "Verify")]),
            Value::object([("exit_code", Value::number(0))]),
        );
    }
    state.close_tools();
    assert_eq!(state.settled_len(), 2);
    let revision = state.tool_revision;
    start(
        &mut state,
        "pending",
        "bash",
        args([("command", "printf later"), ("work", "Verify")]),
    );
    assert_eq!(state.settled_len(), 0);
    assert!(state.tool_revision > revision);
    let working = block(&state.rows()[0]);
    assert!(working.contains("● Verify (3 calls · running)"));
    assert!(working.contains("2 commands completed · 1 running"));
    finish(
        &mut state,
        "pending",
        "bash",
        Value::object([("exit_code", Value::number(0))]),
    );
    let finished = block(&state.rows()[0]);
    assert!(finished.contains("● Verify (3 calls)"));
    assert!(finished.contains("3 commands completed"));
    state.close_tools();
    assert_eq!(state.settled_len(), 3);
}

#[test]
fn a_failed_call_stays_red_and_explicit_after_later_success() {
    let mut state = State::default();
    state.begin_tool_turn();
    done(
        &mut state,
        "failed",
        "read",
        args([("path", "missing.rs"), ("work", "Inspect")]),
        args([("error", "permission denied")]),
    );
    done(
        &mut state,
        "success",
        "bash",
        args([("command", "printf ok"), ("work", "Inspect")]),
        Value::object([("exit_code", Value::number(0))]),
    );
    state.close_tools();
    let rows = state.rows();
    assert_eq!(heading(&rows[0]).spans[0].style, BAD);
    let visible = block(&rows[0]);
    assert!(visible.contains("● Inspect (2 calls · 1 failed)"));
    assert!(visible.contains("1 command completed · 1 failed"));
    assert!(visible.contains("Read missing.rs") && visible.contains("permission denied"));
    assert!(rows[1].is_empty());
}

#[test]
fn a_long_replayed_chapter_keeps_one_compact_heading() {
    let mut state = State::default();
    state.begin_tool_turn();
    for index in 0..10_000 {
        done(
            &mut state,
            &index.to_string(),
            "bash",
            args([("command", "fixture"), ("work", "Replay work")]),
            Value::object([("exit_code", Value::number(0))]),
        );
    }
    state.close_tools();
    state.finish_tool_turn();
    let text = state
        .rows()
        .iter()
        .flat_map(|rows| rows.iter())
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(text.matches("● Replay work (10000 calls)").count(), 1);
    assert!(text.contains("10000 commands completed"));
    assert_eq!(state.settled_len(), state.items.len());
}

#[test]
fn removing_withdrawn_items_rebuilds_chapters_without_merging_execution_turns() {
    let mut state = State::default();
    state.message(Kind::User, "Earlier prompt");
    for turn in 0..2 {
        state.begin_tool_turn();
        for call in 0..(2 - turn) {
            done(
                &mut state,
                &format!("{turn}-{call}"),
                "bash",
                args([("command", "fixture"), ("work", "Inspect")]),
                Value::object([("exit_code", Value::number(0))]),
            );
        }
        state.close_tools();
        state.finish_tool_turn();
    }
    state.items.remove(0);
    state.reindex_chapters();
    state.reset_tool_details();
    let rows = state.rows();
    assert!(block(&rows[0]).contains("● Inspect (2 calls)"));
    assert!(rows[1].is_empty());
    assert!(block(&rows[2]).contains("● Inspect (1 call)"));
}
