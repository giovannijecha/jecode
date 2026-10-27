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
    assert!(header.starts_with(" BASH  fixture command"));
    let summary = visible
        .lines()
        .find(|line| {
            line.trim()
                .strip_prefix("└ Running · ")
                .map(str::trim)
                .unwrap_or_default()
                .strip_suffix('s')
                .is_some_and(|value| value.parse::<f64>().is_ok())
        })
        .unwrap();
    let seconds: f64 = summary
        .trim()
        .strip_prefix("└ Running · ")
        .unwrap()
        .trim_end_matches('s')
        .parse()
        .unwrap();
    assert!(seconds >= 12.0);
    assert!(!header.contains('▰'));
    assert!(!summary.contains(['▰', '▱']));
    // Only the footer may carry the context gauge.
    let rows: Vec<_> = visible.lines().collect();
    assert!(
        rows[..rows.len() - 1]
            .iter()
            .all(|line| !line.contains(['▰', '▱']))
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
    display.scroll(Scroll::Start);
    display.scroll(Scroll::Rows(2));
    step(&mut display, &mut screen, &state, now);
    assert!(screen.visible().starts_with(" WORK  Activity  2 calls"));
    assert!(!screen.visible().contains("  │ one"));
    let generation = state.generation;
    state.toggle_tools();
    step(&mut display, &mut screen, &state, now);
    assert_eq!(state.generation, generation);
    assert!(screen.visible().starts_with(" WORK  Activity  2 calls"));
    assert!(screen.visible().contains("  │ one"));
    assert_eq!(screen.visible().matches(" BASH  first").count(), 1);

    geometry(&mut screen, &mut state, &mut display, 40, 20, now);
    drain(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    // At 40 columns the collapse keycap shortens the expanded title.
    let title = screen.visible().lines().next().unwrap().to_owned();
    assert!(title.starts_with(" WORK  Activity  2 ca"), "{title}");
    assert!(title.trim_end().ends_with("Ctrl+O  collapse"), "{title}");
    assert!(screen.visible().contains("  │ one"));
    state.toggle_tools();
    step(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(100),
    );
    assert!(!screen.visible().contains("  │ one"));
    assert!(screen.history.is_empty());
}

#[test]
fn tail_rows_above_a_growing_work_chapter_never_move_down() {
    for columns in [80, 40] {
        let now = Instant::now();
        let mut state = State {
            width: columns,
            height: 20,
            ..State::default()
        };
        state.message(Kind::Assistant, &"Earlier transcript line.  \n".repeat(25));
        state.begin_tool_turn();
        let mut screen = Screen::new(columns, state.height);
        let mut display = Display::new((columns, state.height));
        let mut previous = usize::MAX;
        let mut observe = |state: &State, phase: &str| {
            step(&mut display, &mut screen, state, now);
            drain(&mut display, &mut screen, state, now);
            let title = if state.items.len() == 2 {
                " BASH  printf one"
            } else {
                " WORK  Inspect sources"
            };
            let current = screen
                .visible()
                .lines()
                .position(|line| line.contains(title))
                .unwrap_or_else(|| panic!("{phase}: work chapter missing\n{}", screen.visible()));
            assert!(
                current <= previous,
                "{columns} columns, {phase}: chapter moved from {previous} to {current}\n{}",
                screen.visible()
            );
            previous = current;
        };
        let begin = |state: &mut State, id: &str, name: &str, args: Value| {
            state.event(Event::ToolStarted {
                id: id.into(),
                name: name.into(),
                arguments: args,
            });
        };
        let done = |state: &mut State, id: &str, name: &str, result: Value| {
            state.event(Event::ToolFinished {
                id: id.into(),
                name: name.into(),
                summary: crate::events::tool_summary(name, &result),
                result,
            });
        };
        let work = "Inspect sources";
        begin(
            &mut state,
            "b1",
            "bash",
            Value::object([
                ("command", Value::string("printf one")),
                ("work", Value::string(work)),
            ]),
        );
        observe(&state, "singleton bash starts");
        done(
            &mut state,
            "b1",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        observe(&state, "singleton bash finishes");
        begin(
            &mut state,
            "r1",
            "read",
            Value::object([
                ("path", Value::string("src/a.rs")),
                ("work", Value::string(work)),
            ]),
        );
        observe(&state, "singleton becomes chapter");
        done(
            &mut state,
            "r1",
            "read",
            Value::object([("content", Value::string("1: source\n"))]),
        );
        observe(&state, "read finishes");
        begin(
            &mut state,
            "r2",
            "read",
            Value::object([
                ("path", Value::string("missing.rs")),
                ("work", Value::string(work)),
            ]),
        );
        observe(&state, "read starts");
        done(
            &mut state,
            "r2",
            "read",
            Value::object([("error", Value::string("permission denied"))]),
        );
        observe(&state, "read fails");
        begin(
            &mut state,
            "b2",
            "bash",
            Value::object([
                ("command", Value::string("printf two")),
                ("work", Value::string(work)),
            ]),
        );
        observe(&state, "bash starts");
        done(
            &mut state,
            "b2",
            "bash",
            Value::object([("exit_code", Value::number(0))]),
        );
        observe(&state, "bash finishes");
        begin(
            &mut state,
            "browser",
            "browser",
            Value::object([
                ("action", Value::string("snapshot")),
                ("tab_id", Value::string("tab-1")),
                ("work", Value::string(work)),
            ]),
        );
        observe(&state, "browser starts");
        done(
            &mut state,
            "browser",
            "browser",
            Value::object([("snapshot_path", Value::string("tmp:browser/snapshot.txt"))]),
        );
        observe(&state, "browser finishes");
        state.close_tools();
        state.finish_tool_turn();
        observe(&state, "turn closes");
        state.begin_tool_turn();
        begin(
            &mut state,
            "next-turn",
            "read",
            Value::object([("path", Value::string("src/next.rs"))]),
        );
        observe(&state, "new turn takes the keycap");
    }
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
            .trim_start_matches(['│', '└'])
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
    display.scroll(Scroll::Start);
    display.scroll(Scroll::Rows(2));
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
