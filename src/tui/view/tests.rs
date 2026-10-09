use super::*;
use crate::tui::state::Kind;
use crate::tui::theme::CURSOR;
use crate::{events::Event, json::Value};

fn text(frame: &Frame) -> String {
    frame
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_steady_caret_follows_a_multiline_draft_when_a_tool_finishes() {
    let mut state = State::default();
    state.editor.insert("first\ncittà🙂");
    state.event(Event::ToolStarted {
        id: "fixture".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("fixture-command"))]),
    });
    let end = frame(&state, "fixture", "folder");
    let (row, column) = end.cursor.unwrap();
    assert_eq!(row, end.composer + 2);
    assert_eq!(column, 9);
    assert_eq!(end.live[row].spans.last().unwrap().style, CURSOR);
    state.editor.cursor = "first\nci".len();
    state.event(Event::ToolFinished {
        id: "fixture".into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([("exit_code", Value::number(0))]),
    });
    let middle = frame(&state, "fixture", "folder");
    let (row, column) = middle.cursor.unwrap();
    assert_eq!(row, middle.composer + 2);
    assert_eq!(column, 4);
    assert_eq!(
        middle.live[row]
            .spans
            .iter()
            .find(|span| span.style == CURSOR)
            .unwrap()
            .text,
        "t"
    );
    assert_eq!(state.editor.text, "first\ncittà🙂");
}

#[test]
fn the_composer_stays_at_the_bottom_below_a_short_transcript() {
    let mut state = State::default();
    state.message(Kind::User, "request");
    state.message(Kind::Assistant, "response");
    state.editor.insert("next draft");
    let frame = frame(&state, "fixture/model", "fixture folder");
    assert_eq!(frame.history.len(), 2);
    assert_eq!(frame.history[1].last().unwrap().plain(), "response");
    assert_eq!(frame.composer, state.height - 4);
    assert_eq!(frame.live[frame.composer - 1], Line::default());
    assert_eq!(frame.live.len(), state.height);
    assert_eq!(frame.live[0].plain(), ">_ Jecode");
    let (row, column) = frame.cursor.unwrap();
    assert!(frame.live[row].plain().contains("next draft"));
    assert_eq!(column, 12);
    assert!(!text(&frame).lines().any(|row| row.trim() == "You"));
}

#[test]
fn pending_tools_and_following_notices_remain_live_until_the_result_arrives() {
    let mut state = State::default();
    state.message(Kind::User, "request");
    state.event(Event::ToolStarted {
        id: "fixture".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("fixture-command"))]),
    });
    state.message(Kind::Notice, "Saved fixture export.");
    let pending = frame(&state, "fixture", "folder");
    assert_eq!(pending.history.len(), 1);
    assert!(
        pending
            .live
            .iter()
            .any(|line| line.plain().contains("running"))
    );
    assert!(
        pending
            .live
            .iter()
            .any(|line| line.plain().contains("Saved fixture export."))
    );
    state.event(Event::ToolFinished {
        id: "fixture".into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([("exit_code", Value::number(0))]),
    });
    let completed = frame(&state, "fixture", "folder");
    assert_eq!(completed.history.len(), 2);
    assert!(
        completed
            .live
            .iter()
            .any(|line| line.plain().contains("fixture-command"))
    );
    assert_eq!(completed.composer, state.height - 5);
}

#[test]
fn help_uses_a_temporary_panel_and_keeps_the_draft() {
    let mut state = State::default();
    state.editor.insert("first\nsecond");
    state.information = Some(crate::tui::information::Information::new(
        "Commands and controls".into(),
        vec![
            ("/export".into(), "Save JSON".into()),
            (
                "PgUp/PgDn and wheel".into(),
                "conversation scrolling".into(),
            ),
        ],
    ));
    let open = frame(&state, "fixture", "folder");
    assert!(!text(&open).lines().any(|row| row.trim() == "/help"));
    assert!(!text(&open).contains("└─"));
    assert!(text(&open).contains("Commands and controls"));
    assert!(text(&open).contains("/export"));
    assert!(text(&open).contains("PgUp/PgDn and wheel"));
    assert!(text(&open).contains("conversation scrolling"));
    assert!(!text(&open).contains("Ctrl+O"));
    assert!(state.items.is_empty());
    assert!(open.cursor.is_none());
    assert!(!text(&open).contains("first"));
    state.information = None;
    let closed = frame(&state, "fixture", "folder");
    assert!(closed.history.is_empty());
    assert!(text(&closed).contains("first"));
    assert_eq!(state.editor.text, "first\nsecond");
}

#[test]
fn the_live_area_and_caret_fit_small_windows_without_truncating_the_draft() {
    let draft = "first\nsecond\nthird\nfourth\nfifth\nsixth";
    for width in [24, 40, 80] {
        for height in [1, 2, 3, 6, 12, 24] {
            let mut state = State {
                width,
                height,
                ..State::default()
            };
            state.editor.insert(draft);
            let frame = frame(&state, "fixture", "folder");
            assert!(frame.live.len() <= height);
            let (row, column) = frame.cursor.unwrap();
            assert!(row < frame.live.len());
            assert!(column < width);
            assert!(
                frame
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= width)
            );
            assert_eq!(state.editor.text, draft);
        }
    }
}

#[test]
fn a_finished_card_stays_live_until_its_final_branch_is_known() {
    let mut state = State::default();
    state.event(Event::ToolStarted {
        id: "fixture".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("fixture-command"))]),
    });
    state.event(Event::ToolFinished {
        id: "fixture".into(),
        name: "bash".into(),
        summary: "exit 0".into(),
        result: Value::object([("exit_code", Value::number(0))]),
    });
    let provisional = frame(&state, "fixture", "folder");
    assert!(provisional.history.is_empty());
    assert!(
        provisional
            .live
            .iter()
            .any(|line| line.plain().starts_with("├─ bash"))
    );
    state.event(Event::Waiting {
        model: "fixture".into(),
    });
    let waiting = frame(&state, "fixture", "folder");
    assert!(waiting.history.is_empty());
    assert!(
        waiting
            .live
            .iter()
            .any(|line| line.plain().starts_with("├─ bash"))
    );
    state.message(Kind::Assistant, "Finished.");
    let settled = frame(&state, "fixture", "folder");
    assert_eq!(settled.history.len(), 2);
    assert!(settled.history[0][0].plain().starts_with("└─ bash"));
    assert_eq!(settled.composer, state.height - 4);
}
