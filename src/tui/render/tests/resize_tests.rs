use super::*;
use crate::tui::state::Item;
use crate::tui::{activity::Activity, display::Display, selector::Selector};
use std::time::{Duration, Instant};

pub(super) fn step(
    display: &mut Display,
    screen: &mut Screen,
    state: &State,
    now: Instant,
) -> String {
    let output = display.update(state, "fixture/model", "fixture directory", now);
    screen.enter();
    assert!(!output.contains('\n') && !output.contains("\x1b[3J"));
    screen.feed(&output);
    output
}

pub(super) fn drain(display: &mut Display, screen: &mut Screen, state: &State, now: Instant) {
    for _ in 0..1000 {
        if !display.needs_draw(state, now) {
            return;
        }
        step(display, screen, state, now);
    }
    panic!("reconstruction did not finish");
}

pub(super) fn geometry(
    screen: &mut Screen,
    state: &mut State,
    display: &mut Display,
    width: usize,
    height: usize,
    now: Instant,
) {
    screen.resize(width, height);
    state.width = width;
    state.height = height;
    display.resized((width, height), now);
}

pub(super) fn transcript(screen: &Screen) -> String {
    screen.history.join("\n") + "\n" + &screen.visible()
}

// Read the owned conversation through emitted viewport pages, excluding the
// composer. This checks retained rows without relying on native scrollback.
pub(super) fn owned_transcript(
    display: &mut Display,
    screen: &mut Screen,
    state: &State,
    now: Instant,
) -> String {
    use crate::tui::viewport::Scroll;
    let capacity = crate::tui::view::capacity(state, "fixture/model", "fixture directory");
    assert!(capacity > 0 && capacity + 1 < state.height);
    display.scroll(Scroll::Start);
    step(display, screen, state, now);
    let mut result = Vec::new();
    for _ in 0..20_000 {
        let status: String = screen.rows[capacity].iter().collect();
        if !status.contains("Back to bottom") && !status.contains("Bottom") {
            result.extend(
                screen.rows[..capacity]
                    .iter()
                    .map(|row| row.iter().collect::<String>()),
            );
            display.scroll(Scroll::End);
            step(display, screen, state, now);
            return result.join("\n");
        }
        result.push(screen.rows[0].iter().collect());
        display.scroll(Scroll::Rows(1));
        step(display, screen, state, now);
    }
    panic!("owned transcript did not reach the last page");
}

pub(super) fn assert_clean_history(screen: &Screen) {
    assert!(
        !screen
            .history
            .iter()
            .any(|line| line.contains("Ask anything")
                || line.contains("fixture/model ·")
                || line.contains("queued")
                || line.contains("Working ·")
                || line.contains("Thinking ·")
                || line.contains("kept draft")
                || line.contains("· running"))
    );
}

#[test]
fn resizing_during_streaming_then_tools_and_finalization_never_saves_provisional_rows() {
    let now = Instant::now();
    let mut state = State {
        height: 16,
        ..State::default()
    };
    let mut screen = Screen::new(state.width, state.height);
    screen.feed("outside shell output\r\n");
    let mut display = Display::new((state.width, state.height));
    state.message(Kind::User, "Inspect the project.");
    state.message(Kind::Assistant, &"retained paragraph\n".repeat(30));
    state.editor.insert("kept draft\nsecond line🙂");
    state.editor.cursor = "kept draft\nsec".len();
    let draft = state.editor.clone();
    state.queue.push("follow-up queued".into()).unwrap();
    state.activity = Some(Activity::new());
    state.event(Event::Streaming {
        text: "A provisional answer.".into(),
    });
    step(&mut display, &mut screen, &state, now);
    geometry(&mut screen, &mut state, &mut display, 40, 12, now);
    let preview = step(&mut display, &mut screen, &state, now);
    assert!(!preview.contains("\x1b[3J"));
    assert!(screen.visible().contains("provisional"));
    state.event(Event::Message {
        text: "A completed answer.".into(),
    });
    start(&mut state, "inspect-command");
    let settled = now + Duration::from_millis(75);
    assert!(!step(&mut display, &mut screen, &state, settled).contains("\x1b[3J"));
    drain(&mut display, &mut screen, &state, settled);
    assert_eq!(state.editor, draft);
    assert_eq!(state.queue.messages.len(), 1);
    finish(&mut state, "inspect-command", "captured output\n");
    step(&mut display, &mut screen, &state, settled);
    state.event(Event::Message {
        text: "All done.".into(),
    });
    state.activity = None;
    let final_frame = step(&mut display, &mut screen, &state, settled);
    assert!(!final_frame.contains("\x1b[3J"));
    drain(&mut display, &mut screen, &state, settled);
    let text = owned_transcript(&mut display, &mut screen, &state, settled);
    assert!(!text.contains("outside shell output"));
    assert!(!text.contains("provisional"));
    assert_eq!(text.matches("retained paragraph").count(), 30);
    assert_eq!(text.matches("A completed answer.").count(), 1);
    assert_eq!(text.matches("inspect-command").count(), 1);
    assert_eq!(text.matches("captured output").count(), 1);
    assert_eq!(text.matches("All done.").count(), 1);
    assert_clean_history(&screen);
    assert!(step(&mut display, &mut screen, &state, settled).is_empty());
}

#[test]
fn a_second_resize_discards_unwritten_rows_and_a_large_replay_keeps_input_responsive() {
    let now = Instant::now();
    let mut state = State {
        height: 18,
        ..State::default()
    };
    let mut text = String::new();
    for index in 0..1600 {
        text.push_str(&format!("source-{index:04} {}\n", "word ".repeat(12)));
    }
    state.message(Kind::Assistant, &text);
    let mut display = Display::new((state.width, state.height));
    let mut screen = Screen::new(state.width, state.height);
    step(&mut display, &mut screen, &state, now);
    drain(&mut display, &mut screen, &state, now);
    geometry(&mut screen, &mut state, &mut display, 44, 14, now);
    step(&mut display, &mut screen, &state, now);
    let first_deadline = now + Duration::from_millis(75);
    let first_chunk = step(&mut display, &mut screen, &state, first_deadline);
    assert!(!first_chunk.contains("\x1b[3J"));
    assert!(
        first_chunk.len() < 40 * 1024,
        "output must be bounded, including the live viewport"
    );
    assert!(display.needs_draw(&state, first_deadline));
    state.editor.insert("kept draft");
    state.queue.push("follow-up queued".into()).unwrap();
    let second_resize = first_deadline + Duration::from_millis(1);
    geometry(&mut screen, &mut state, &mut display, 70, 20, second_resize);
    assert!(!step(&mut display, &mut screen, &state, second_resize).contains("\x1b[3J"));
    assert!(screen.visible().contains("kept draft"));
    assert!(!display.needs_draw(&state, second_resize + Duration::from_millis(74)));
    let second_deadline = second_resize + Duration::from_millis(75);
    step(&mut display, &mut screen, &state, second_deadline);
    drain(&mut display, &mut screen, &state, second_deadline);
    let transcript = owned_transcript(&mut display, &mut screen, &state, second_deadline);
    for index in 0..1600 {
        assert_eq!(transcript.matches(&format!("source-{index:04}")).count(), 1);
    }
    assert_eq!(state.editor.text, "kept draft");
    assert_eq!(state.queue.messages.len(), 1);
    assert_clean_history(&screen);
    assert!(step(&mut display, &mut screen, &state, second_deadline).is_empty());
}

#[test]
fn hidden_local_commands_and_new_conversations_survive_later_reconstructions() {
    let now = Instant::now();
    let mut state = State {
        height: 10,
        ..State::default()
    };
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    state.message(Kind::Assistant, &"previous conversation\n".repeat(30));
    step(&mut display, &mut screen, &state, now);
    state.clear();
    state.items.push(Item::Local {
        command: "/model fixture/new".into(),
        result: None,
        kind: Kind::Notice,
        details: vec![],
    });
    step(&mut display, &mut screen, &state, now);
    geometry(&mut screen, &mut state, &mut display, 44, 9, now);
    step(&mut display, &mut screen, &state, now);
    if let Item::Local { result, .. } = &mut state.items[0] {
        *result = Some("Model set to fixture/new".into());
    }
    state.message(Kind::Assistant, "Current conversation.");
    let deadline = now + Duration::from_millis(75);
    step(&mut display, &mut screen, &state, deadline);
    drain(&mut display, &mut screen, &state, deadline);
    let text = transcript(&screen);
    assert!(!text.contains("previous conversation"));
    assert!(!text.contains("/model fixture/new"));
    assert!(!text.contains("Model set to fixture/new"));
    assert_eq!(text.matches("Current conversation.").count(), 1);
    assert_eq!(text.matches("Jecode").count(), 1);
    assert!(!text.contains("Applying selection"));
    assert_eq!(state.items.len(), 2);
}

#[test]
fn selectors_preserve_query_selection_and_the_separate_draft_in_very_short_windows() {
    let now = Instant::now();
    let mut state = State::default();
    state.editor.insert("separate draft🙂");
    let draft = state.editor.clone();
    state.selector = Some(Selector::settings(
        "fixture/model",
        crate::effort::Effort::Default,
    ));
    state.selector.as_mut().unwrap().selected = 3;
    let mut screen = Screen::new(state.width, state.height);
    let mut display = Display::new((state.width, state.height));
    step(&mut display, &mut screen, &state, now);
    for height in [6, 4, 3, 2, 18] {
        geometry(&mut screen, &mut state, &mut display, 36, height, now);
        step(
            &mut display,
            &mut screen,
            &state,
            now + Duration::from_millis(75),
        );
        assert!(screen.visible().contains("4. Close"));
        assert_eq!(state.selector.as_ref().unwrap().selected, 3);
        assert_eq!(state.editor, draft);
    }
    let models: Vec<_> = (0..12)
        .map(|index| crate::openrouter::Model {
            id: format!("fixture/model-{index}"),
            name: format!("Model {index}"),
            prompt_price: None,
            completion_price: None,
            efforts: vec![crate::effort::Effort::Default],
        })
        .collect();
    let mut menu = Selector::models(&models, false, "fixture/model");
    menu.editor.insert("model");
    menu.refresh();
    menu.selected = 9;
    let selected = menu.options[menu.filtered[menu.selected].index]
        .name
        .clone();
    state.selector = Some(menu);
    geometry(&mut screen, &mut state, &mut display, 40, 3, now);
    step(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(75),
    );
    assert!(screen.visible().contains(&selected));
    assert!(screen.visible().contains("› model"));
    assert_eq!(state.selector.as_ref().unwrap().editor.text, "model");
    assert_eq!(state.editor, draft);
}

#[test]
fn tiny_geometries_suspend_output_and_later_restore_the_latest_stream_and_draft() {
    let now = Instant::now();
    let mut state = State::default();
    let mut display = Display::new((80, 24));
    let mut screen = Screen::new(80, 24);
    step(&mut display, &mut screen, &state, now);
    for size in [(1, 24), (80, 1), (1, 1)] {
        display.resized(size, now);
        state.width = size.0;
        state.height = size.1;
        state.event(Event::Streaming {
            text: "Current streamed response.".into(),
        });
        state.editor.insert("kept draft");
        assert!(
            display
                .update(
                    &state,
                    "fixture/model",
                    "fixture directory",
                    now + Duration::from_secs(1)
                )
                .is_empty()
        );
        assert!(!display.needs_draw(&state, now + Duration::from_secs(1)));
        assert_eq!(
            display.wait(&state, now + Duration::from_secs(1)),
            Duration::from_millis(30)
        );
    }
    geometry(&mut screen, &mut state, &mut display, 80, 24, now);
    step(
        &mut display,
        &mut screen,
        &state,
        now + Duration::from_millis(75),
    );
    assert!(screen.visible().contains("Current streamed response."));
    assert!(screen.visible().contains("kept draftkept draftkept draft"));
    assert_clean_history(&screen);
}
