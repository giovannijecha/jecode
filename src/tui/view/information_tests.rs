use super::composer_frame as frame;
use super::*;
use crate::tui::{
    Feedback,
    information::Information,
    state::Kind,
    theme::{BAD, MUTED, USER_BACKGROUND, WARNING},
};

fn info() -> Information {
    Information::new(
        "Commands and controls".into(),
        (0..30)
            .map(|index| {
                (
                    format!("/command-{index}"),
                    format!("Description {index} with wrapped text on a narrow terminal"),
                )
            })
            .collect(),
    )
}

fn plain(state: &State) -> String {
    frame(state, "fixture", "fixture")
        .live
        .iter()
        .map(Line::plain)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn information_scrolls_by_visual_rows_and_reaches_every_entry_after_resize() {
    let mut state = State {
        width: 40,
        height: 9,
        information: Some(info()),
        ..State::default()
    };
    state.editor.replace("unsent draft".into());
    assert!(plain(&state).contains("/command-0"));
    scroll_information(&mut state, 34);
    assert_eq!(state.information.as_ref().unwrap().offset, 5);
    scroll_information(&mut state, 35);
    assert!(plain(&state).contains("Description 29"));
    state.width = 80;
    state.height = 12;
    assert!(plain(&state).contains("Description 29"));
    scroll_information(&mut state, 38);
    assert!(plain(&state).contains("Description 28"));
    scroll_information(&mut state, 36);
    assert_eq!(state.information.as_ref().unwrap().offset, 0);
    assert!(plain(&state).contains("/command-0"));
    scroll_information(&mut state, 40);
    assert_eq!(state.information.as_ref().unwrap().offset, 1);
    assert_eq!(state.editor.text, "unsent draft");
}

#[test]
fn information_fits_tiny_viewports_and_keeps_one_shaded_surface_and_close_control() {
    for width in [2, 5, 12, 24, 80] {
        for height in [1, 2, 3, 6, 12, 24] {
            let mut state = State {
                width,
                height,
                information: Some(info()),
                ..State::default()
            };
            state.editor.replace("preserved\ndraft".into());
            let rendered = frame(&state, "fixture", "fixture");
            assert!(rendered.live.len() <= height);
            assert!(rendered.cursor.is_none());
            assert!(rendered.history.is_empty());
            assert!(
                rendered
                    .live
                    .iter()
                    .all(|line| line.background == Some(USER_BACKGROUND))
            );
            assert!(
                rendered
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= width)
            );
            if width >= 5 {
                assert!(plain(&state).contains("Esc"));
            }
            scroll_information(&mut state, 35);
            state.width = 80;
            state.height = 24;
            assert!(plain(&state).contains("Description 29"));
            assert_eq!(state.editor.text, "preserved\ndraft");
        }
    }
}

#[test]
fn all_feedback_is_left_aligned_italic_and_unshaded_with_severity_colors() {
    for (kind, style) in [
        (Kind::Notice, MUTED),
        (Kind::Warning, WARNING),
        (Kind::Error, BAD),
    ] {
        let state = State {
            notice: Some(Feedback::result(kind, "A brief result")),
            ..State::default()
        };
        let rendered = frame(&state, "fixture", "fixture");
        assert_eq!(rendered.live[0].plain(), "A brief result");
        assert_eq!(rendered.live[0].background, None);
        assert_eq!(
            rendered.live[0].spans.last().unwrap().style,
            format!("3;{style}")
        );
        assert!(!plain(&state).contains('●'));
        assert!(!plain(&state).contains('•'));
    }
}

#[test]
fn long_errors_wrap_without_losing_the_actionable_text_or_moving_the_draft() {
    let mut state = State {
        width: 40,
        notice: Some(Feedback::result(
            Kind::Error,
            "Could not write /a/very/long/session/path: read-only. Session kept in memory.",
        )),
        ..State::default()
    };
    state.editor.replace("kept draft".into());
    state.editor.cursor = 2;
    let rendered = frame(&state, "fixture", "fixture");
    assert!(plain(&state).contains("read-only"));
    let notices: Vec<_> = rendered
        .live
        .iter()
        .take_while(|line| !line.plain().starts_with('─'))
        .collect();
    assert!(notices.len() > 1);
    assert!(
        notices
            .iter()
            .all(|line| line.background.is_none() && !line.plain().starts_with(' '))
    );
    assert!(
        rendered.live[rendered.cursor.unwrap().0]
            .plain()
            .contains("kept draft")
    );
    assert_eq!(state.editor.cursor, 2);
}
