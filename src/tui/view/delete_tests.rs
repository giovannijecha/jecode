use super::*;
use crate::{
    sessions::Summary,
    tui::{
        selector::{Purpose, Selector},
        state::Kind,
        theme::BAD,
    },
};

fn sessions() -> Vec<Summary> {
    (0..10)
        .map(|index| Summary {
            id: format!("1234-56-{index}"),
            title: format!("Saved request {index}"),
            updated: index,
            backend: "openrouter".into(),
            model: "fixture/model".into(),
        })
        .collect()
}

#[test]
fn resume_marks_deletion_in_the_existing_list_and_keeps_confirmation_visible_in_small_viewports() {
    let entries = sessions();
    let mut state = State::default();
    state.editor.insert("kept draft");
    let mut menu = Selector::sessions(&entries, &entries[3].id);
    menu.selected = 3;
    menu.toggle_delete();
    state.selector = Some(menu);
    let full = frame(&state, "fixture/model", "fixture directory");
    let rendered = full
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("─ Resume · current folder"));
    // The marked row keeps its place and names the pending deletion.
    let marked = full
        .live
        .iter()
        .find(|line| line.plain().contains("› Saved request 3"))
        .unwrap();
    let name = marked.spans.iter().find(|span| span.text.contains("Saved"));
    assert_eq!(name.unwrap().style, BAD);
    // The question in BAD replaces the keys, with its own keycaps.
    let confirmation = |frame: &Frame| {
        frame
            .live
            .iter()
            .find(|line| line.plain().trim_start().starts_with("Del"))
            .cloned()
    };
    let question = confirmation(&full).unwrap();
    assert_eq!(
        question.plain().trim_end(),
        "  Delete this conversation and its files?    Enter  delete    Esc  cancel"
    );
    assert_eq!(
        question
            .spans
            .iter()
            .find(|span| !span.text.trim().is_empty())
            .unwrap()
            .style,
        BAD
    );
    assert!(!rendered.contains("↑↓  move"));
    let marked = |line: &Line| {
        line.plain().starts_with("› ")
            && focused(line)
            && line.spans.iter().any(|span| span.style == BAD)
    };
    assert!(full.live.iter().any(marked));
    for width in [8, 18, 40, 80] {
        for height in [1, 2, 4, 6, 10, 20] {
            state.width = width;
            state.height = height;
            let fitted = frame(&state, "fixture/model", "fixture directory");
            assert!(fitted.live.len() <= height);
            assert!(
                fitted
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= width)
            );
            // The search field and the confirmation come first, then the
            // title and the marked row; below eighty columns the confirmation
            // wraps its keys onto a second row.
            let rows: Vec<_> = fitted.live.iter().map(Line::plain).collect();
            assert!(fitted.cursor.is_some(), "{width}x{height} {rows:?}");
            if height >= 2 {
                assert!(confirmation(&fitted).is_some(), "{width}x{height} {rows:?}");
            }
            if width >= 18 && height >= 4 + usize::from(width < 80) {
                assert!(fitted.live.iter().any(marked), "{width}x{height} {rows:?}");
            } else if height >= 5 {
                // The focused option row, beside the scrollbar.
                let option = |line: &Line| focused(line) && line.plain().ends_with(['┃', '│']);
                assert!(fitted.live.iter().any(option), "{width}x{height} {rows:?}");
            }
            assert_eq!(state.editor.text, "kept draft");
        }
    }
}

#[test]
fn resume_exposes_delete_control_and_shows_loading_error_and_success_in_the_same_list() {
    let entries = sessions();
    let mut state = State {
        selector: Some(Selector::sessions(&entries, &entries[3].id)),
        ..State::default()
    };
    let rendered = frame(&state, "fixture/model", "fixture")
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains(" Ctrl+D  delete"));
    state.height = 8;
    state.selector.as_mut().unwrap().toggle_delete();
    if let Purpose::Sessions { working, .. } = &mut state.selector.as_mut().unwrap().purpose {
        *working = true;
    }
    let busy = frame(&state, "fixture/model", "fixture")
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(busy.matches("Deleting conversation…").count(), 1);
    assert!(!busy.contains(" Enter  delete"));
    assert!(busy.contains(" Ctrl+Q  quit after cleanup"));
    state.selector = Some(Selector::sessions(&entries, &entries[3].id));
    for (kind, message) in [
        (Kind::Error, "Session is already open"),
        (Kind::Notice, "Deleted saved request"),
    ] {
        state.notice = Some(crate::tui::Feedback::result(kind, message));
        let rendered = frame(&state, "fixture/model", "fixture")
            .live
            .iter()
            .map(Line::plain)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(rendered.contains(message));
        assert!(state.selector.is_some());
    }
}
