use super::resize_tests::{drain, geometry, step, transcript};
use super::*;
use crate::tui::display::Display;
use std::time::{Duration, Instant};

#[test]
fn a_current_deletion_erases_the_owned_conversation_without_using_native_history() {
    let now = Instant::now();
    let mut state = State {
        height: 12,
        ..State::default()
    };
    let mut display = Display::new((state.width, state.height));
    let mut screen = Screen::new(state.width, state.height);
    for number in 0..20 {
        state.message(Kind::User, &format!("Deleted request {number}"));
        state.message(Kind::Assistant, "Deleted answer");
    }
    step(&mut display, &mut screen, &state, now);
    drain(&mut display, &mut screen, &state, now);
    assert!(transcript(&screen).contains("Deleted request"));
    assert!(screen.history.is_empty());
    state.items.clear();
    state.editor.insert("kept draft");
    state.queue.push("follow-up".into()).unwrap();
    state.generation += 1;
    state.changed();
    display.reset((state.width, state.height));
    drain(&mut display, &mut screen, &state, now);
    let visible = transcript(&screen);
    assert!(!visible.contains("Deleted request") && !visible.contains("Deleted answer"));
    assert!(visible.contains("kept draft") && visible.contains("follow-up"));
    geometry(&mut screen, &mut state, &mut display, 40, 10, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(75),
    );
    assert!(!transcript(&screen).contains("Deleted"));
}
