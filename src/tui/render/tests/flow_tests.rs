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
    assert!(header.contains("● Bash(fixture command)"));
    let summary = visible
        .lines()
        .find(|line| line.contains("⎿ Running…"))
        .unwrap();
    let seconds: f64 = summary
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
    assert!(
        visible
            .lines()
            .any(|line| line.starts_with("••••  Running tools  ·  "))
    );
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
    state.begin_tool_turn();
    start(&mut state, "first");
    finish(&mut state, "first", "one\n\nthree\nfour\nfive\n");
    start(&mut state, "second");
    finish(&mut state, "second", "second result");
    state.finish_tool_turn();
    state.message(
        Kind::Assistant,
        &(0..50)
            .map(|i| format!("Later paragraph {i:02}.  \n"))
            .collect::<String>(),
    );
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &mut screen, &state, now);
    display.scroll(Scroll::Reveal(1));
    step(&mut display, &mut screen, &state, now);
    assert!(screen.visible().starts_with("● Bash(first)"));
    assert!(!screen.visible().contains("    one"));
    let generation = state.generation;
    state.toggle_tools();
    step(&mut display, &mut screen, &state, now);
    assert_eq!(state.generation, generation);
    assert!(screen.visible().starts_with("● Bash(first)"));
    assert!(screen.visible().contains("    one"));
    assert_eq!(screen.visible().matches("● Bash(first)").count(), 1);

    geometry(&mut screen, &mut state, &mut display, 40, 20, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(screen.visible().starts_with("● Bash(first)"));
    assert!(screen.visible().contains("    one"));
    state.toggle_tools();
    step(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(!screen.visible().contains("    one"));
    assert!(screen.history.is_empty());
}

#[test]
fn resizing_inside_expanded_tool_output_keeps_the_logical_source_line() {
    fn first_source_line(screen: &Screen) -> String {
        screen
            .visible()
            .lines()
            .next()
            .unwrap()
            .trim_start()
            .split(':')
            .next()
            .unwrap()
            .to_owned()
    }

    let now = Instant::now();
    let mut state = State::default();
    state.begin_tool_turn();
    start(&mut state, "expanded output");
    let output = (0..30)
        .map(|i| format!("Output line {i:03}: one two three four five six seven eight nine.\n"))
        .collect::<String>();
    finish(&mut state, "expanded output", &output);
    state.finish_tool_turn();
    state.message(Kind::Assistant, "Later response.");
    state.toggle_tools();
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &mut screen, &state, now);
    display.scroll(Scroll::Reveal(1));
    display.scroll(Scroll::Rows(6));
    step(&mut display, &mut screen, &state, now);
    let anchor = first_source_line(&screen);
    assert!(anchor.starts_with("Output line "), "{}", screen.visible());
    geometry(&mut screen, &mut state, &mut display, 40, 20, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert_eq!(first_source_line(&screen), anchor);
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
    assert_eq!(first_source_line(&screen), anchor);
}
