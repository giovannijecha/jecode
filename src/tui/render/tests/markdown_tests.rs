use super::resize_tests::{assert_clean_history, drain, geometry, owned_transcript, step};
use super::*;
use crate::tui::{activity::Activity, display::Display};
use std::time::{Duration, Instant};

#[test]
fn streamed_tables_resize_and_finalize_without_saving_provisional_layouts() {
    let now = Instant::now();
    let mut state = State {
        height: 18,
        ..State::default()
    };
    state.message(Kind::Assistant, &"retained paragraph\n".repeat(30));
    state.editor.insert("kept draft");
    state.queue.push("follow-up queued").unwrap();
    state.activity = Some(Activity::new());
    let mut display = Display::new((state.width, state.height));
    let mut screen = Screen::new(state.width, state.height);
    state.event(Event::Streaming {
        text: "| Name | Note |\n|---|---|\n| OLD | LIVE |".into(),
    });
    step(&mut display, &mut screen, &state, now);
    drain(&mut display, &mut screen, &state, now);
    assert!(screen.visible().contains("LIVE"));
    assert!(!screen.history.iter().any(|row| row.contains("OLD")));
    geometry(&mut screen, &mut state, &mut display, 9, 18, now);
    let narrow = now + Duration::from_millis(75);
    step(&mut display, &mut screen, &state, narrow);
    drain(&mut display, &mut screen, &state, narrow);
    assert!(screen.visible().contains("Note:"));
    state.event(Event::Message {
        text: "| Name | Note |\n|---|---|\n| FINAL_A | complete |\n| FINAL_B | ready |".into(),
    });
    state.activity = None;
    let deadline = now + Duration::from_millis(75);
    step(&mut display, &mut screen, &state, deadline);
    drain(&mut display, &mut screen, &state, deadline);
    geometry(&mut screen, &mut state, &mut display, 80, 18, deadline);
    let wide = deadline + Duration::from_millis(75);
    step(&mut display, &mut screen, &state, wide);
    drain(&mut display, &mut screen, &state, wide);
    let output = owned_transcript(&mut display, &mut screen, &state, wide);
    assert_eq!(output.matches("retained paragraph").count(), 30);
    assert_eq!(output.matches("FINAL_A").count(), 1);
    assert_eq!(output.matches("FINAL_B").count(), 1);
    assert!(!output.contains("OLD"));
    assert!(!output.contains("LIVE"));
    assert!(!output.contains("|---|"));
    assert_eq!(state.editor.text, "kept draft");
    assert_eq!(state.queue.messages.len(), 1);
    assert_clean_history(&screen);
    assert!(step(&mut display, &mut screen, &state, wide).is_empty());
}
