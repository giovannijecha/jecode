//! Bounded, opt-in UI measurements with synthetic text and an isolated session.
use super::{
    App,
    display::Display,
    state::{Kind, State},
    viewport::Scroll,
};
use crate::{
    agent::Agent, backend::openrouter::OpenRouter, events::Event, json::Value,
    test_support::Directory, tools::Tools,
};
use std::{hint::black_box, time::Instant};

fn text(bytes: usize) -> String {
    let paragraph = "A **synthetic response** with `code`, a [reference](https://example.test), and ordinary words to reproduce wrapping in a terminal.\n\n";
    paragraph.repeat(bytes.div_ceil(paragraph.len()))[..bytes].into()
}

fn state() -> State {
    State {
        width: 120,
        height: 40,
        ..State::default()
    }
}

fn drain(display: &mut Display, state: &State) -> usize {
    let mut bytes = 0;
    for _ in 0..4096 {
        bytes +=
            black_box(display.update(state, "fixture/model", "fixture directory", Instant::now()))
                .len();
        if !display.needs_draw(state, Instant::now()) {
            return bytes;
        }
    }
    panic!("synthetic layout did not complete within 4096 passes");
}

fn samples(name: &str, size: usize, mut work: impl FnMut() -> usize) {
    for _ in 0..3 {
        black_box(work());
    }
    let mut times = Vec::new();
    for repeat in 0..21 {
        let started = Instant::now();
        let bytes = work();
        let us = started.elapsed().as_secs_f64() * 1e6;
        times.push(us);
        println!("RAW,{name},{size},{repeat},{us:.3},{bytes}");
    }
    times.sort_by(f64::total_cmp);
    println!(
        "SUMMARY,{name},{size},21,{:.3},{:.3},{:.3}",
        times[0], times[10], times[20]
    );
}

fn tool(state: &mut State) {
    state.event(Event::ToolStarted {
        id: "fixture".into(),
        name: "bash".into(),
        arguments: Value::object([("command", Value::string("fixture"))]),
    });
    state.event(Event::ToolFinished {
        id: "fixture".into(),
        name: "bash".into(),
        summary: "complete".into(),
        result: Value::object([
            ("stdout", Value::string("synthetic output\n")),
            ("exit_code", Value::number(0)),
        ]),
    });
    state.close_tools();
}

#[test]
#[ignore = "synthetic release UI benchmark; no network, personal configuration or terminal writes"]
fn composer_stream_scroll_and_capture() {
    println!(
        "META,120x40; 3 warmups; 21 samples; elapsed microseconds; UI work only; no terminal writes"
    );
    for size in [64, 65536, 262144] {
        let mut state = state();
        state.editor.replace(text(size));
        let mut display = Display::new((state.width, state.height));
        drain(&mut display, &state);
        samples("typing_draft", size, || {
            assert!(state.editor.insert("x"));
            drain(&mut display, &state)
        });
    }

    let mut state = state();
    state.begin_tool_turn();
    tool(&mut state);
    state.finish_tool_turn();
    for index in 0..5000 {
        state.message(
            if index % 2 == 0 {
                Kind::User
            } else {
                Kind::Assistant
            },
            &text(180),
        );
    }
    let mut display = Display::new((state.width, state.height));
    drain(&mut display, &state);
    samples("scroll_history", 5000, || {
        display.scroll(Scroll::Rows(-3));
        drain(&mut display, &state)
    });
    samples("toggle_tools", 5000, || {
        state.toggle_tools();
        drain(&mut display, &state)
    });

    for size in [65536, 262144] {
        let mut state = self::state();
        state.message(Kind::User, "Synthetic request.");
        let mut source = text(size);
        state.event(Event::Streaming {
            text: source.clone(),
        });
        let mut display = Display::new((state.width, state.height));
        drain(&mut display, &state);
        samples("stream_unchanged", size, || {
            state.event(Event::Streaming {
                text: source.clone(),
            });
            drain(&mut display, &state)
        });
        samples("stream_append", size, || {
            source.push_str(" Added **formatted** text to the active response.\n");
            state.event(Event::Streaming {
                text: source.clone(),
            });
            drain(&mut display, &state)
        });
        state.event(Event::Message { text: source });
        drain(&mut display, &state);
    }

    let home = Directory::new();
    let directory = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(directory.path()).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let current = App::new(agent, super::tests::config(&directory), None);
    let payload = "x".repeat(4096);
    for index in 0..5000 {
        current
            .archive
            .messages
            .lock()
            .unwrap()
            .push(Value::object([
                (
                    "role",
                    Value::string(if index % 2 == 0 { "user" } else { "assistant" }),
                ),
                ("content", Value::string(&payload)),
            ]));
    }
    current.agent.as_ref().unwrap().save_session().unwrap();
    let mut current = current;
    samples("capture_input", 5000, || {
        current.state.editor.insert("x");
        current.capture_input();
        black_box(&current);
        0
    });
    current.persist_input(true);
    assert_eq!(
        current
            .persistence
            .as_ref()
            .unwrap()
            .snapshot()
            .input
            .draft
            .text,
        current.state.editor.text
    );
}
