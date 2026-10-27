use super::composer_frame as frame;
use super::*;
use crate::tui::{Feedback, information::Information, state::Kind, theme::USER_SURFACE};

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
    assert!(plain(&state).contains("› unsent draft"));
    // A page is the three body rows left below the composer and its title,
    // above the blank row and the keys.
    scroll_information(&mut state, 34);
    assert_eq!(state.information.as_ref().unwrap().offset, 3);
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
fn information_fits_tiny_viewports_on_the_terminal_surface_and_keeps_its_close_control() {
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
            assert!(rendered.live.iter().all(|line| {
                line.background.is_none()
                    && line
                        .spans
                        .iter()
                        .all(|span| span.background != Some(USER_SURFACE))
            }));
            assert!(
                rendered
                    .live
                    .iter()
                    .all(|line| text::cells(&line.plain()) <= width)
            );
            if width >= 5 {
                assert!(
                    plain(&state).contains("Esc") || height == 1,
                    "{width}x{height}:
{}",
                    plain(&state)
                );
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
fn notices_take_the_status_row_without_moving_the_draft() {
    let mut idle = State::default();
    idle.editor.insert("kept draft");
    let before = frame(&idle, "fixture", "fixture");
    for (kind, glyph) in [
        (Kind::Notice, "✓"),
        (Kind::Warning, "▲"),
        (Kind::Error, "✕"),
    ] {
        let mut state = State::default();
        state.editor.insert("kept draft");
        state.notice = Some(Feedback::result(kind, "A brief result"));
        let rendered = frame(&state, "fixture", "fixture");
        assert_eq!(
            rendered.live[0].plain(),
            format!(" {glyph}  A brief result ")
        );
        assert_eq!(rendered.live.len(), before.live.len());
        assert_eq!(rendered.cursor, before.cursor);
        assert_eq!(rendered.live[1..], before.live[1..]);
    }
}

#[test]
fn long_errors_shorten_within_the_status_row() {
    let mut state = State {
        width: 40,
        notice: Some(Feedback::result(
            Kind::Error,
            "Could not write: read-only · session kept in memory",
        )),
        ..State::default()
    };
    state.editor.replace("kept draft".into());
    state.editor.cursor = 2;
    let rendered = frame(&state, "fixture", "fixture");
    let status = rendered.live[0].plain();
    assert_eq!(status, " ✕  Could not write   read-only");
    assert!(rendered.live[1].plain().starts_with('─'));
    assert!(
        rendered.live[rendered.cursor.unwrap().0]
            .plain()
            .contains("kept draft")
    );
    assert_eq!(state.editor.cursor, 2);
}
