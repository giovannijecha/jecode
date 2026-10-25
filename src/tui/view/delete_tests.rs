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
    assert!(rendered.contains("Resume · current folder"));
    // The marked row keeps its place and names the pending deletion.
    let marked = full
        .live
        .iter()
        .find(|line| line.plain().contains("› Saved request 3"))
        .unwrap();
    let name = marked.spans.iter().find(|span| span.text.contains("Saved"));
    assert_eq!(name.unwrap().style, BAD);
    assert!(
        rendered.contains("owned outputs and temporary files"),
        "{rendered}"
    );
    assert!(rendered.contains("unsent input is kept"));
    assert!(rendered.contains("Delete this conversation?"));
    assert!(rendered.contains(" Enter  delete"));
    assert!(
        full.live
            .iter()
            .any(|line| focused(line) && line.spans.iter().any(|span| span.style == BAD))
    );
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
            // The search field and the confirmation come first, then the row.
            if height >= 4 {
                assert!(fitted.live.iter().any(focused), "{width}x{height}");
            }
            if width >= 18 && height >= 2 {
                assert!(
                    fitted
                        .live
                        .iter()
                        .any(|line| line.plain().contains("Delete"))
                );
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
