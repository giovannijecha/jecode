//! A long conversation that keeps working while its layout is rebuilt: the
//! visible page must follow new output, scrolling and resizing at once.
use super::resize_tests::{geometry, step};
use super::*;
use crate::tui::{display::Display, transcript::Layout};
use std::time::{Duration, Instant};

fn read(state: &mut State, id: &str, lines: usize) {
    state.event(Event::ToolStarted {
        id: id.into(),
        name: "read".into(),
        arguments: Value::object([("path", Value::string(format!("src/{id}.rs")))]),
    });
    finish(state, id, lines);
}

fn finish(state: &mut State, id: &str, lines: usize) {
    let content = (0..lines)
        .map(|line| format!("{id}:{line:04} fn synthetic() {{}}\n"))
        .collect::<String>();
    state.event(Event::ToolFinished {
        id: id.into(),
        name: "read".into(),
        summary: "complete".into(),
        result: Value::object([
            ("content", Value::string(content)),
            ("lines_returned", Value::number(lines)),
            ("offset", Value::number(1)),
        ]),
    });
}

/// Settled turns of a request, two reads and an answer; every 25th answer is
/// long enough to take many layout passes on its own.
fn history(turns: usize) -> State {
    let mut state = State {
        width: 100,
        height: 30,
        ..State::default()
    };
    for turn in 0..turns {
        state.message(
            Kind::User,
            &format!("Request {turn}: fix the failing case."),
        );
        state.begin_tool_turn();
        read(&mut state, &format!("turn{turn}a"), 40);
        read(&mut state, &format!("turn{turn}b"), 40);
        state.finish_tool_turn();
        let paragraphs = if turn % 25 == 12 { 600 } else { 4 };
        let answer = (0..paragraphs)
            .map(|index| {
                format!("Paragraph {turn}.{index} explains a finding in enough ordinary words to wrap.\n\n")
            })
            .collect::<String>();
        state.message(Kind::Assistant, &answer);
    }
    state
}

fn settle(display: &mut Display, screen: &mut Screen, state: &State, now: Instant) -> usize {
    for frame in 0..10_000 {
        if !display.needs_draw(state, now) {
            return frame;
        }
        step(display, screen, state, now);
    }
    panic!("layout did not finish while new events kept arriving");
}

#[test]
fn newly_settled_long_answer_keeps_a_bounded_live_tail() {
    let answer = "A long answer with **emphasis** and enough words to wrap.\n\n".repeat(5_000);
    let mut state = State::default();
    state.event(Event::Streaming {
        text: answer.clone(),
    });
    let mut layout = Layout::default();
    layout.prepare(&state, state.width, 64);
    layout.prepare_pending(&state, 256);
    assert!(!layout.pending_ready(&state));

    // Finalizing the stream makes it settled, but the first frame must not
    // render the entire answer through the mutable tail in one call.
    state.event(Event::Message { text: answer });
    layout.prepare(&state, state.width, 64);
    assert!(!layout.ready(&state));
    layout.prepare_pending(&state, 256);
    assert!(layout.pending_blocks()[0].len() <= 514);
    assert!(!layout.pending_ready(&state));

    state.event(Event::Streaming {
        text: "LIVE-TAIL while the prior answer settles".into(),
    });
    state.editor.insert("draft remains editable");
    let now = Instant::now();
    let mut display = Display::new((state.width, state.height));
    let mut screen = Screen::new(state.width, state.height);
    step(&mut display, &mut screen, &state, now);
    let visible = screen.visible();
    assert!(visible.contains("LIVE-TAIL"));
    assert!(visible.contains("draft remains editable"));
}

#[test]
fn a_rebuild_keeps_the_page_live_and_tool_events_cannot_restart_it() {
    let start = Instant::now();
    let mut state = history(60);
    let mut display = Display::new((state.width, state.height));
    let mut screen = Screen::new(state.width, state.height);
    settle(&mut display, &mut screen, &state, start);

    // A resize rewraps every block; the work under way continues meanwhile.
    geometry(&mut screen, &mut state, &mut display, 72, 30, start);
    let now = start + Duration::from_millis(80);
    state.begin_tool_turn();
    state.event(Event::Streaming {
        text: "LIVE-STREAM-ONE is under way".into(),
    });
    state.editor.insert("typed during the rebuild");
    step(&mut display, &mut screen, &state, now);
    assert!(display.needs_draw(&state, now), "the rebuild is incomplete");
    assert!(screen.visible().contains("LIVE-STREAM-ONE"));
    assert!(screen.visible().contains("typed during the rebuild"));
    assert!(!screen.visible().contains("Reflowing conversation"));

    // Tool events every frame never restart the rebuild of a long answer.
    state.event(Event::Message {
        text: "LIVE-STREAM-ONE is under way".into(),
    });
    let mut frames = 0;
    for call in 0.. {
        let id = format!("live{call}");
        state.event(Event::ToolStarted {
            id: id.clone(),
            name: "read".into(),
            arguments: Value::object([("path", Value::string(format!("src/{id}.rs")))]),
        });
        step(&mut display, &mut screen, &state, now);
        assert!(screen.visible().contains(&format!("src/{id}.rs")), "{id}");
        finish(&mut state, &id, 3);
        step(&mut display, &mut screen, &state, now);
        frames += 2;
        if !display.needs_draw(&state, now) {
            break;
        }
        assert!(
            frames < 2_000,
            "the rebuild restarted with every tool event"
        );
    }

    // Scrolling moves the page during the next rebuild too.
    geometry(&mut screen, &mut state, &mut display, 100, 30, now);
    let later = now + Duration::from_millis(80);
    step(&mut display, &mut screen, &state, later);
    let bottom = screen.visible();
    display.scroll(crate::tui::viewport::Scroll::Page(true));
    step(&mut display, &mut screen, &state, later);
    assert!(display.needs_draw(&state, later));
    assert_ne!(screen.visible(), bottom, "the wheel moved the page");
    display.scroll(crate::tui::viewport::Scroll::End);
    settle(&mut display, &mut screen, &state, later);
    let transcript = screen.visible();
    assert!(transcript.contains(&format!("src/live{}.rs", frames / 2 - 1)));
}

/// Release measurement of frame cost while a long conversation keeps working:
/// cargo test --release --locked --offline active_tests::active_long_history_frames -- --ignored --nocapture
#[test]
#[ignore = "synthetic release UI measurement; no network, configuration or terminal writes"]
fn active_long_history_frames() {
    println!("META,turns,frames,p50_us,p95_us,max_us,stale_frames,first_visible_frames");
    for turns in [200, 1000] {
        let start = Instant::now();
        let mut state = history(turns);
        let mut display = Display::new((state.width, state.height));
        let mut screen = Screen::new(state.width, state.height);
        settle(&mut display, &mut screen, &state, start);
        let mut now = start;
        let mut times = Vec::new();
        let mut stale_frames = 0;
        let mut first_visible = Vec::new();
        state.begin_tool_turn();
        for cycle in 0..60 {
            if cycle % 20 == 5 {
                let width = if state.width == 100 { 80 } else { 100 };
                geometry(&mut screen, &mut state, &mut display, width, 30, now);
                now += Duration::from_millis(80);
            }
            let marker = format!("CYCLE-{cycle}-STREAM");
            let mut shown = None;
            for (frame, words) in (1..=6).enumerate() {
                state.event(Event::Streaming {
                    text: format!("{marker} {}", "word ".repeat(words * 20)),
                });
                if cycle % 10 == 7 && frame == 2 {
                    display.scroll(crate::tui::viewport::Scroll::Rows(-3));
                }
                if cycle % 10 == 7 && frame == 4 {
                    display.scroll(crate::tui::viewport::Scroll::End);
                }
                let started = Instant::now();
                step(&mut display, &mut screen, &state, now);
                times.push(started.elapsed().as_micros());
                stale_frames += usize::from(display.needs_draw(&state, now));
                if shown.is_none() && screen.visible().contains(&marker) {
                    shown = Some(frame + 1);
                }
            }
            first_visible.push(shown.unwrap_or(usize::MAX));
            state.event(Event::Message {
                text: format!("{marker} done"),
            });
            let id = format!("cycle{cycle}");
            read(&mut state, &id, 30);
            let started = Instant::now();
            step(&mut display, &mut screen, &state, now);
            times.push(started.elapsed().as_micros());
        }
        times.sort_unstable();
        let percentile = |p: usize| times[(times.len() - 1) * p / 100];
        println!(
            "SUMMARY,{turns},{},{},{},{},{stale_frames},{}",
            times.len(),
            percentile(50),
            percentile(95),
            times.last().unwrap(),
            first_visible.iter().max().unwrap()
        );
    }
}
