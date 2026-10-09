use super::resize_tests::{drain, geometry, step};
use super::*;
use crate::tui::{activity::Activity, display::Display, state::Item, viewport::Scroll};
use std::time::{Duration, Instant};

#[test]
fn live_tool_time_updates_without_events_then_freezes_on_completion() {
    let now = Instant::now();
    let mut state = State {
        activity: Some(Activity::new()),
        ..State::default()
    };
    start(&mut state, "fixture command");
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    step(&mut display, &mut screen, &state, now);
    let revision = state.revision;
    if let Item::Tool { presentation, .. } = &mut state.items[0] {
        presentation.started = Some(Instant::now() - Duration::from_secs(12));
    }
    step(&mut display, &mut screen, &state, now);
    let visible = screen.visible();
    let header = visible
        .lines()
        .find(|line| line.contains("fixture command"))
        .unwrap();
    let seconds: f64 = header
        .rsplit('·')
        .next()
        .unwrap()
        .trim()
        .trim_end_matches('s')
        .parse()
        .unwrap();
    assert!(seconds >= 12.0);
    assert!(!header.contains('•'));
    assert_eq!(visible.matches("••••").count(), 1);
    assert!(visible.lines().any(|line| line.starts_with("••••  ")));
    assert_eq!(state.revision, revision);

    finish(&mut state, "fixture command", "done");
    state.close_tools();
    state.activity = None;
    step(&mut display, &mut screen, &state, now);
    let completed = screen.visible();
    let Item::Tool { presentation, .. } = &mut state.items[0] else {
        panic!("missing tool")
    };
    let frozen = presentation.duration().unwrap();
    presentation.started = Some(Instant::now() - Duration::from_secs(90));
    assert_eq!(presentation.duration(), Some(frozen));
    assert!(step(&mut display, &mut screen, &state, now).is_empty());
    assert_eq!(screen.visible(), completed);
    assert!(!completed.contains("••••"));
}

#[test]
fn cached_old_tool_expansion_reflows_without_losing_the_reading_anchor() {
    let now = Instant::now();
    let mut state = State::default();
    start(&mut state, "first");
    finish(&mut state, "first", "one\n\nthree\nfour\nfive\n");
    start(&mut state, "second");
    finish(&mut state, "second", "second result");
    state.message(
        Kind::Assistant,
        &(0..50)
            .map(|i| format!("Later paragraph {i:02}.\n"))
            .collect::<String>(),
    );
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &mut screen, &state, now);
    display.scroll(Scroll::Reveal(1));
    step(&mut display, &mut screen, &state, now);
    assert!(screen.visible().starts_with("├─ bash  first"));
    assert!(!screen.visible().contains("five"));
    let generation = state.generation;
    state.select_tool(Some(0));
    state.toggle_tool();
    step(&mut display, &mut screen, &state, now);
    assert_eq!(state.generation, generation);
    assert!(screen.visible().starts_with("├─ bash  first"));
    assert!(screen.visible().contains("│    five"));
    assert_eq!(screen.visible().matches("├─ bash  first").count(), 1);

    geometry(&mut screen, &mut state, &mut display, 40, 20, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(screen.visible().starts_with("├─ bash  first"));
    assert!(screen.visible().contains("│    five"));
    state.toggle_tool();
    step(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(!screen.visible().contains("five"));
    assert!(screen.history.is_empty());
}

#[test]
fn resizing_inside_expanded_tool_output_keeps_the_logical_source_line() {
    let now = Instant::now();
    let mut state = State::default();
    start(&mut state, "expanded output");
    let output = (0..30)
        .map(|i| format!("Output line {i:03}: one two three four five six seven eight nine.\n"))
        .collect::<String>();
    finish(&mut state, "expanded output", &output);
    state.message(Kind::Assistant, "Later response.");
    state.select_tool(Some(0));
    state.toggle_tool();
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &mut screen, &state, now);
    display.scroll(Scroll::Reveal(1));
    display.scroll(Scroll::Rows(6));
    step(&mut display, &mut screen, &state, now);
    assert!(
        screen
            .visible()
            .lines()
            .next()
            .unwrap()
            .contains("Output line 005:")
    );
    geometry(&mut screen, &mut state, &mut display, 40, 20, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(
        screen
            .visible()
            .lines()
            .next()
            .unwrap()
            .contains("Output line 005:")
    );
    geometry(
        &mut screen,
        &mut state,
        &mut display,
        80,
        24,
        now + Duration::from_millis(200),
    );
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(300),
    );
    assert!(
        screen
            .visible()
            .lines()
            .next()
            .unwrap()
            .contains("Output line 005:")
    );
}
