//! Reference chips across successive frames of the app's one renderer: composer
//! movement and deletion, a sent request in its user box, scrolling, resizing and
//! both palettes. Every incremental screen must equal a fresh paint of the same
//! state, so no stale text, chip surface or caret cell survives.

use super::resize_tests::{drain, geometry, step};
use super::*;
use crate::attachments::Attachment;
use crate::tui::{
    display::Display, state::Kind, terminal::Key, test_screen::Rgb, theme, viewport::Scroll,
};
use std::time::{Duration, Instant};

const LONG: &str = "@\"docs/a very long folder name/with a long file name for the transcript.md\"";

fn rgb(sgr: &str) -> Rgb {
    let start = sgr.find("48;2;").unwrap() + 5;
    let values: Vec<u8> = sgr[start..]
        .split(';')
        .take(3)
        .map(|value| value.trim_end_matches('m').parse().unwrap())
        .collect();
    (values[0], values[1], values[2])
}

fn press(state: &mut State, code: u16) {
    state.editor.compose_key(Key {
        code,
        modifiers: 0,
        character: 0,
    });
}

/// Draws the state with `display`, as the app does after input, then checks the
/// screen against a fresh one.
fn settled(display: &mut Display, screen: &mut Screen, state: &State, now: Instant) {
    step(display, screen, state, now);
    drain(display, screen, state, now);
    let mut fresh_display = Display::new((state.width, state.height));
    let mut fresh = Screen::new(state.width, state.height);
    step(&mut fresh_display, &mut fresh, state, now);
    drain(&mut fresh_display, &mut fresh, state, now);
    assert_eq!(screen.visible(), fresh.visible());
    assert_eq!(screen.backs, fresh.backs, "{}", screen.visible());
    let caret = rgb(&theme::CURSOR.to_string());
    let carets = screen
        .backs
        .iter()
        .flatten()
        .filter(|back| **back == Some(caret));
    assert_eq!(carets.count(), 1, "one caret cell\n{}", screen.visible());
}

/// Where `needle` first shows, as (row, column).
fn find(screen: &Screen, needle: &str) -> Option<(usize, usize)> {
    screen.rows.iter().enumerate().find_map(|(row, cells)| {
        let text: String = cells
            .iter()
            .map(|ch| if *ch == '\u{a0}' { ' ' } else { *ch })
            .collect();
        text.find(needle)
            .map(|byte| (row, text[..byte].chars().count()))
    })
}

/// Every cell of the chip ` needle ` shows the chip surface, the caret aside.
fn filled(screen: &Screen, needle: &str) {
    let (row, column) = find(screen, &format!(" {needle} ")).unwrap_or_else(|| {
        panic!("{needle} is not on screen\n{}", screen.visible());
    });
    let chip = Some(rgb(&theme::BADGE_IDLE.to_string()));
    let caret = Some(rgb(&theme::CURSOR.to_string()));
    for cell in column..column + needle.chars().count() + 2 {
        let back = screen.backs[row][cell];
        assert!(
            back == chip || back == caret,
            "{needle} at {cell}\n{}",
            screen.visible()
        );
    }
}

fn image() -> Attachment {
    Attachment {
        id: "att-fixture".into(),
        name: "shot.png".into(),
        media: "image/png".into(),
        size: 10,
        width: Some(1),
        height: Some(1),
        pages: None,
    }
}

#[test]
fn composer_chips_repaint_cleanly_through_movement_deletion_and_resize() {
    for light in [false, true] {
        theme::set_light(light);
        for (width, other) in [(44, 100), (100, 44)] {
            let mut state = State {
                width,
                height: 18,
                ..State::default()
            };
            state.editor.insert("Check ");
            state.editor.attach(image());
            state.editor.compose(
                &format!(" @AGENTS.md and $accessibility, then {LONG} ok"),
                false,
            );
            let mut display = Display::new((state.width, state.height));
            let mut screen = Screen::new(state.width, state.height);
            let now = Instant::now();
            settled(&mut display, &mut screen, &state, now);
            for chip in ["1 Image", "@AGENTS.md", "$accessibility"] {
                filled(&screen, chip);
            }
            // Repeated movement across every chip, then back.
            while state.editor.cursor > 0 {
                press(&mut state, 37);
                settled(&mut display, &mut screen, &state, now);
            }
            while state.editor.cursor < state.editor.text.len() {
                press(&mut state, 39);
                settled(&mut display, &mut screen, &state, now);
            }
            // Backspace after one reference and Delete before another.
            state.editor.cursor = state.editor.text.find(", then").unwrap();
            press(&mut state, 8);
            settled(&mut display, &mut screen, &state, now);
            assert!(!screen.visible().contains("$accessibility"));
            filled(&screen, "@AGENTS.md");
            state.editor.cursor = state.editor.text.find('@').unwrap();
            press(&mut state, 46);
            settled(&mut display, &mut screen, &state, now);
            assert!(
                !screen.visible().contains("@AGENTS"),
                "{}",
                screen.visible()
            );
            filled(&screen, "1 Image");
            press(&mut state, 8);
            press(&mut state, 8);
            settled(&mut display, &mut screen, &state, now);
            assert!(!screen.visible().contains("Image"));
            // The same renderer after a resize.
            geometry(&mut screen, &mut state, &mut display, other, 18, now);
            settled(
                &mut display,
                &mut screen,
                &state,
                now + Duration::from_secs(2),
            );
            assert_eq!(state.editor.text.matches('@').count(), 1);
        }
    }
    theme::set_light(false);
}

#[test]
fn sent_requests_keep_their_chips_in_the_user_box_across_frames() {
    let request = format!("Check [1# Image] with @AGENTS.md, $review and {LONG} please");
    for light in [false, true] {
        theme::set_light(light);
        let mut state = State {
            width: 100,
            height: 16,
            ..State::default()
        };
        state.message(Kind::User, &request);
        state.message(Kind::Assistant, &"answer row  \n".repeat(30));
        let mut display = Display::new((state.width, state.height));
        let mut screen = Screen::new(state.width, state.height);
        let mut now = Instant::now();
        settled(&mut display, &mut screen, &state, now);
        assert!(
            find(&screen, "@AGENTS.md").is_none(),
            "scrolled out of view"
        );
        for (width, shortened) in [(100, false), (44, true), (100, false)] {
            if width != state.width {
                now += Duration::from_secs(2);
                geometry(&mut screen, &mut state, &mut display, width, 16, now);
                now += Duration::from_secs(2);
                settled(&mut display, &mut screen, &state, now);
            }
            display.scroll(Scroll::Start);
            step(&mut display, &mut screen, &state, now);
            for chip in ["1 Image", "@AGENTS.md", "$review"] {
                filled(&screen, chip);
            }
            let (row, column) = find(&screen, "Check").unwrap();
            assert_eq!(column, 2, "the request hangs at column 2");
            assert_eq!(screen.rows[row][0], '›');
            let surface = Some(rgb(&theme::USER_SURFACE.to_string()));
            assert_eq!(screen.backs[row][column], surface);
            assert_eq!(screen.backs[row][width - 3], surface, "the full-row box");
            let tail = if shortened {
                "@\"docs/a very long folder name/with"
            } else {
                LONG
            };
            filled_from(&screen, tail, shortened);
            display.scroll(Scroll::End);
            settled(&mut display, &mut screen, &state, now);
            assert!(find(&screen, "@AGENTS.md").is_none());
        }
    }
    theme::set_light(false);
}

/// The long reference shows whole, or shortened with `…` inside its chip.
fn filled_from(screen: &Screen, start: &str, shortened: bool) {
    let (row, column) = find(screen, start).unwrap_or_else(|| panic!("{}", screen.visible()));
    let text: String = screen.rows[row].iter().collect();
    let chip = Some(rgb(&theme::BADGE_IDLE.to_string()));
    let cells: Vec<_> = (column - 1..screen.rows[row].len())
        .take_while(|cell| screen.backs[row][*cell] == chip)
        .collect();
    let shown: String = cells.iter().map(|cell| screen.rows[row][*cell]).collect();
    assert_eq!(shown.contains('…'), shortened, "{text}");
    if !shortened {
        assert_eq!(shown.replace('\u{a0}', " "), format!(" {LONG} "));
    }
}
