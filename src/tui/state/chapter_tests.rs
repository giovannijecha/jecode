use super::*;
use crate::events::Event;
use crate::tui::{
    line::Line,
    theme::{BADGE_BAD, BADGE_IDLE, BADGE_TEXT, STRONG},
};

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

/// A title row opens on a ` NAME ` badge; hung rows open on the gutter.
fn titled(row: &Line) -> bool {
    let plain = row.plain();
    plain.starts_with(' ') && plain.chars().nth(1).is_some_and(char::is_uppercase)
}

fn heading(rows: &[Line]) -> &Line {
    rows.iter()
        .find(|row| titled(row))
        .expect("chapter heading")
}

#[test]
fn a_live_mixed_chapter_never_loses_rows_and_keeps_its_last_activity() {
    for columns in [80, 40] {
        let mut state = State {
            width: columns,
            ..State::default()
        };
        let mut prior = [0; 2];
        let mut check = |state: &State, phase: &str| {
            for (slot, keycap) in [false, true].into_iter().enumerate() {
                let mut renderer = rows::Rows::default();
                let height: usize = (0..state.items.len())
                    .map(|index| {
                        renderer
                            .render_at(&state.items, index, columns, keycap && index == 0)
                            .len()
                    })
                    .sum();
                assert!(
                    height >= prior[slot],
                    "{columns} columns, {phase}, keycap {keycap}: {height} < {}\n{}",
                    prior[slot],
                    block(&state.rows().into_iter().flatten().collect::<Vec<_>>())
                );
                prior[slot] = height;
            }
        };
        let work = "Inspect sources";
        state.begin_tool_turn();
        start(
            &mut state,
            "bash-one",
            "bash",
            args([("command", "printf one"), ("work", work)]),
        );
        check(&state, "first bash starts");
        finish(
            &mut state,
            "bash-one",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        check(&state, "first bash finishes");
        start(
            &mut state,
            "read-one",
            "read",
            args([("path", "src/a.rs"), ("work", work)]),
        );
        check(&state, "read starts and singleton becomes chapter");
        finish(
            &mut state,
            "read-one",
            "read",
            Value::object([("content", Value::string("1: source\n"))]),
        );
        check(&state, "read finishes");
        assert!(
            block(&state.rows()[1]).contains("READ src/a.rs"),
            "{}",
            block(&state.rows()[1])
        );
        start(
            &mut state,
            "read-bad",
            "read",
            args([("path", "missing.rs"), ("work", work)]),
        );
        check(&state, "second read starts");
        finish(
            &mut state,
            "read-bad",
            "read",
            args([("error", "permission denied")]),
        );
        check(&state, "second read fails");
        let visible = block(&state.rows().into_iter().flatten().collect::<Vec<_>>());
        assert!(visible.contains("READ missing.rs"), "{visible}");
        assert_eq!(visible.matches("READ missing.rs").count(), 1);
        assert_eq!(visible.matches("READ src/a.rs").count(), 1);
        start(
            &mut state,
            "bash-two",
            "bash",
            args([("command", "printf two"), ("work", work)]),
        );
        check(&state, "second bash starts");
        finish(
            &mut state,
            "bash-two",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        check(&state, "second bash finishes");
        start(
            &mut state,
            "browser",
            "browser",
            args([("action", "snapshot"), ("tab_id", "tab-1"), ("work", work)]),
        );
        check(&state, "browser starts");
        finish(
            &mut state,
            "browser",
            "browser",
            args([("snapshot_path", "tmp:browser/snapshot.txt")]),
        );
        check(&state, "browser finishes");
        let activity = block(&state.rows()[4]);
        assert!(activity.contains("BROWSER snapshot tab-1"), "{activity}");
        state.close_tools();
        state.finish_tool_turn();
        check(&state, "turn closes");
        assert_eq!(block(&state.rows()[4]), activity);
    }
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
    assert_eq!(heading(&rows[0]).plain(), " WORK  Inspect app  5 calls");
    let summary = block(&rows[0]);
    for fact in [
        "1 file read",
        "1 web call",
        "1 screenshot",
        "1 command",
        "1 process call",
    ] {
        assert!(summary.contains(fact), "missing {fact}: {summary}");
    }
    assert!(!summary.contains("What happened") && !summary.contains("BASH"));
    assert!(block(&rows[1]).contains("BASH printf example"));
    assert!(rows[2..4].iter().all(Vec::is_empty));
    assert!(block(&rows[4]).contains("BROWSER screenshot"));

    assert_eq!(state.toggle_tools(), Some((true, 5)));
    let opened = state.rows();
    for (index, target) in [
        " READ  src/main.rs",
        " BASH  printf example",
        " WEB  fetch",
        " BROWSER  screenshot",
        " PROCESS  list",
    ]
    .into_iter()
    .enumerate()
    {
        assert!(block(&opened[index]).contains(target), "{target}");
    }
    let bash = &opened[1];
    let first = bash
        .iter()
        .position(|row| row.plain() == "  │ first")
        .unwrap();
    assert_eq!(bash[first + 1].plain(), "  │ ");
    assert_eq!(bash[first + 2].plain(), "  │ last");
    assert_eq!(state.toggle_tools(), Some((false, 5)));
    let closed = state.rows();
    assert!(block(&closed[1]).contains("BASH printf example"));
    assert!(closed[2..4].iter().all(Vec::is_empty));
    assert!(block(&closed[4]).contains("BROWSER screenshot"));
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
    assert_eq!(heading(&rows[0]).plain(), " WORK  Audit  2 calls");
    assert!(block(&rows[1]).contains("READ src/a.rs"));
    assert_eq!(heading(&rows[2]).plain(), " WORK  Activity  2 calls");
    assert!(block(&rows[3]).contains("READ src/a.rs"));
    assert!(heading(&rows[4]).plain().starts_with(" READ  src/a.rs"));
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
        .filter_map(|rows| rows.iter().find(|row| titled(row)))
        .map(Line::plain)
        .collect();
    assert_eq!(
        headings
            .iter()
            .filter(|text| text.starts_with(" READ  "))
            .count(),
        6
    );
    for expected in [" WRITE  ", " UPDATE  ", " CHECKPOINT  ", " SKILL  "] {
        assert!(
            headings.iter().any(|text| text.contains(expected)),
            "{headings:?}"
        );
    }
    assert!(headings.iter().all(|text| !text.contains("  2 calls")));
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
    assert_eq!(heading(&rows[0]).plain(), " WORK  Inspect  2 calls");
    assert!(block(&rows[1]).contains("READ old-1.rs"));
    assert_eq!(heading(&rows[2]).plain(), " WORK  Inspect  2 calls");
    assert!(block(&rows[3]).contains("READ new-1.rs"));
    assert_eq!(state.toggle_tools(), Some((true, 2)));
    let opened = state.rows();
    assert!(!block(&opened[0]).contains(" READ  old-0.rs"));
    assert!(!block(&opened[1]).contains(" READ  old-1.rs"));
    assert!(block(&opened[2]).contains(" READ  new-0.rs"));
    assert!(block(&opened[3]).contains(" READ  new-1.rs"));
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
    // The running state lives on the WORK badge; the title names no state.
    let rows = state.rows();
    let title = heading(&rows[0]);
    assert_eq!(title.spans[0].background, Some(BADGE_IDLE));
    assert_eq!(title.spans[1].text, "WORK");
    assert_ne!(title.spans[1].style, STRONG.bold());
    let working = block(&rows[0]);
    assert!(working.contains(" WORK  Verify  3 calls\n"), "{working}");
    assert!(!working.contains("calls · running"));
    assert!(working.contains("2 commands · 1 running"));
    finish(
        &mut state,
        "pending",
        "bash",
        Value::object([("exit_code", Value::number(0))]),
    );
    let finished = block(&state.rows()[0]);
    assert!(finished.contains(" WORK  Verify  3 calls"));
    let rows = state.rows();
    assert_eq!(heading(&rows[0]).spans[0].style, STRONG.bold());
    assert_eq!(heading(&rows[0]).spans[0].background, Some(BADGE_IDLE));
    assert!(finished.contains("3 commands"));
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
    assert_eq!(heading(&rows[0]).spans[0].style, BADGE_TEXT.bold());
    assert_eq!(heading(&rows[0]).spans[0].background, Some(BADGE_BAD));
    let visible = block(&rows[0]);
    assert!(visible.contains(" WORK  Inspect  2 calls · 1 failed"));
    assert!(visible.contains("1 command"));
    assert!(!visible.contains("1 command · 1 failed"));
    assert!(visible.contains("READ missing.rs") && visible.contains("permission denied"));
    assert!(block(&rows[1]).contains("BASH printf ok"));
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
    assert_eq!(text.matches(" WORK  Replay work  10000 calls").count(), 1);
    assert!(text.contains("10000 commands"));
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
    assert!(block(&rows[0]).contains(" WORK  Inspect  2 calls"));
    assert!(block(&rows[1]).contains("BASH fixture"));
    assert!(block(&rows[2]).contains(" BASH  fixture"));
}
