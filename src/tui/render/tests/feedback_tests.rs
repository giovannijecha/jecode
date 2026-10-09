use super::resize_tests::{drain, geometry, owned_transcript, step, transcript};
use super::*;
use crate::tui::{Feedback, display::Display, information::Information, state::Item};
use std::time::{Duration, Instant};

#[test]
fn feedback_and_information_panels_are_erased_and_never_return_in_resized_history() {
    let now = Instant::now();
    let mut state = State {
        height: 12,
        ..State::default()
    };
    state.message(Kind::Assistant, &"Saved conversation row\n".repeat(30));
    state.items.push(Item::Local {
        command: "/help".into(),
        result: Some("Historical help output".into()),
        kind: Kind::Notice,
        details: vec![("old key".into(), "old detail".into())],
    });
    state.items.push(Item::Text {
        kind: Kind::Error,
        text: "Historical UI error".into(),
    });
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    step(&mut display, &mut screen, &state, now);
    drain(&mut display, &mut screen, &state, now);
    for index in 0..4 {
        state.notice = Some(Feedback::at(
            Kind::Notice,
            format!("Temporary result {index}"),
            now,
        ));
        step(&mut display, &mut screen, &state, now);
        assert!(
            screen
                .visible()
                .contains(&format!("Temporary result {index}"))
        );
        state.message(Kind::Assistant, "Next saved row");
        step(&mut display, &mut screen, &state, now);
        assert!(!screen.history.join("\n").contains("Temporary result"));
    }
    assert!(state.expire_feedback(now + Duration::from_secs(5)));
    step(&mut display, &mut screen, &state, now);
    assert!(!transcript(&screen).contains("Temporary result"));
    state.information = Some(Information::new(
        "Temporary information".into(),
        (0..25)
            .map(|index| (format!("key {index}"), format!("detail {index}")))
            .collect(),
    ));
    step(&mut display, &mut screen, &state, now);
    assert!(screen.visible().contains("Temporary information"));
    state.information = None;
    state.copy_notice = Some(Feedback::at(Kind::Notice, "Temporary copy result", now));
    step(&mut display, &mut screen, &state, now);
    state.feedback_input(true);
    step(&mut display, &mut screen, &state, now);
    for (width, height) in [(40, 9), (80, 24)] {
        geometry(&mut screen, &mut state, &mut display, width, height, now);
        let settled = now + Duration::from_millis(75);
        step(&mut display, &mut screen, &state, settled);
        drain(&mut display, &mut screen, &state, settled);
        let text = owned_transcript(&mut display, &mut screen, &state, settled);
        for hidden in ["Temporary", "Historical", "old detail", "key 0"] {
            assert!(!text.contains(hidden), "{hidden} returned after resize");
        }
        assert_eq!(text.matches("Saved conversation row").count(), 30);
        assert_eq!(text.matches("Next saved row").count(), 4);
    }
}
