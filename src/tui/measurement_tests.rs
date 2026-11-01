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

/// Saved messages for settled turns with prose, code, a table and three tools
/// with large results; every 50th answer is very long. Lines are distinct so
/// scrolling repaints rows as a real conversation does.
pub(super) fn mixed_messages(turns: usize) -> Vec<Value> {
    let message = |role: &str, content: String| {
        Value::object([
            ("role", Value::string(role)),
            ("content", Value::string(content)),
        ])
    };
    let mut messages = Vec::new();
    for turn in 0..turns {
        messages.push(message(
            "user",
            format!("Request {turn}: review the module and fix the failing case."),
        ));
        let ids: Vec<_> = (0..3).map(|index| format!("call-{turn}-{index}")).collect();
        let calls = ["read", "bash", "edit"]
            .into_iter()
            .zip(&ids)
            .map(|(name, id)| {
                let arguments = Value::object([
                    ("path", Value::string(format!("src/module_{turn}.rs"))),
                    ("command", Value::string(format!("cargo test case_{turn}"))),
                ]);
                crate::test_support::tool_call(id, name, arguments)
            })
            .collect();
        messages.push(Value::object([
            ("role", Value::string("assistant")),
            ("content", Value::Null),
            ("tool_calls", Value::Array(calls)),
        ]));
        let output = (0..400)
            .map(|line| format!("{turn}:{line:04}: fn synthetic_{line}() -> usize {{ {line} }}\n"))
            .collect::<String>();
        for id in &ids {
            let result = Value::object([
                ("stdout", Value::string(&output)),
                ("content", Value::string(&output)),
                ("exit_code", Value::number(0)),
            ]);
            messages.push(Value::object([
                ("role", Value::string("tool")),
                ("tool_call_id", Value::string(id)),
                ("content", Value::string(result.encode())),
            ]));
        }
        let paragraphs = if turn % 50 == 25 { 400 } else { 6 };
        let mut answer = String::new();
        for paragraph in 0..paragraphs {
            answer.push_str(&format!(
                "Paragraph {turn}.{paragraph} explains **finding {paragraph}** in `module_{turn}`, \
                 compares it with [the reference](https://example.test/{turn}) and keeps \
                 ordinary words long enough to wrap across terminal rows.\n\n"
            ));
        }
        answer.push_str("```rust\n");
        for line in 0..40 {
            answer.push_str(&format!(
                "fn handler_{turn}_{line}(input: &str) -> Result<Vec<u8>, String> {{ Ok(input.bytes().rev().collect()) }}\n"
            ));
        }
        answer.push_str("```\n\n| Case | Before | After | Notes |\n| --- | ---: | ---: | --- |\n");
        for row in 0..12 {
            answer.push_str(&format!(
                "| case {turn}.{row} | {} ms | {} ms | synthetic **row** with `code` |\n",
                row * 7,
                row * 3
            ));
        }
        if turn + 1 == turns {
            answer.push_str("\nFINAL-ANSWER-MARK\n");
        }
        messages.push(message("assistant", answer));
    }
    messages
}

/// An application that resumed a saved mixed conversation.
fn mixed_app(turns: usize, home: &Directory, directory: &Directory) -> App {
    let agent = || {
        let mut agent = Agent::new(
            OpenRouter::fixture("http://127.0.0.1:1".into()),
            Tools::new(directory.path()).unwrap(),
        );
        agent.enable_sessions(home.path()).unwrap();
        agent
    };
    let saved = agent();
    saved
        .archive()
        .messages
        .lock()
        .unwrap()
        .extend(mixed_messages(turns));
    saved.save_session().unwrap();
    let id = saved.sessions().unwrap().snapshot().id.clone();
    drop(saved);
    let mut app = App::new(agent(), super::tests::config(directory), None);
    app.resume_session(&id);
    app
}

#[test]
#[ignore = "synthetic release UI benchmark; no network, personal configuration or terminal writes"]
fn mixed_history_scroll() {
    println!("META,resumed mixed turns; 3 warmups; 21 samples; elapsed microseconds; VT bytes");
    for turns in [200, 1000] {
        let (home, directory) = (Directory::new(), Directory::new());
        let mut app = mixed_app(turns, &home, &directory);
        scroll_samples(&mut app, turns);
    }
}

fn require_settled_document(document: &crate::sessions::Document) -> Result<(), &'static str> {
    if document.pending.active
        || document.input.question.is_some()
        || !document.input.queued.is_empty()
    {
        return Err("The copied journal must contain a settled conversation");
    }
    Ok(())
}

fn require_settled_journal(store: &crate::sessions::Store, id: &str) -> Result<(), &'static str> {
    let document = store
        .read(id)
        .map_err(|_| "The copied journal could not be read")?;
    require_settled_document(&document)
}

fn resume_for_measurement(app: &mut App, id: &str) -> Result<(), &'static str> {
    app.resume_session(id);
    // A returned collaborator can be ready even when the saved turn is settled.
    // Polling the UI must never continue that work during a measurement.
    app.collaboration_auto_resume = false;
    if app.worker.is_some()
        || app.agent.is_none()
        || app
            .persistence
            .as_ref()
            .is_none_or(|handle| handle.id() != id)
    {
        return Err("The copied journal could not be resumed for measurement");
    }
    Ok(())
}

/// Scrolls a copy of one saved conversation in an isolated home and prints
/// only timings and byte counts. Set `JECODE_MEASURE_JOURNAL` to a session
/// journal and `JECODE_MEASURE_DIRECTORY` to the folder it belongs to.
#[test]
#[ignore = "local release UI benchmark of a copied journal; prints no conversation text"]
fn own_history_scroll() {
    let (Some(journal), Some(workspace)) = (
        std::env::var_os("JECODE_MEASURE_JOURNAL"),
        std::env::var_os("JECODE_MEASURE_DIRECTORY"),
    ) else {
        println!("SKIPPED,set JECODE_MEASURE_JOURNAL and JECODE_MEASURE_DIRECTORY");
        return;
    };
    let journal = std::path::PathBuf::from(journal);
    let id = journal.file_stem().unwrap().to_string_lossy().into_owned();
    let home = Directory::new();
    let mut agent = Agent::new(
        OpenRouter::fixture("http://127.0.0.1:1".into()),
        Tools::new(std::path::Path::new(&workspace)).unwrap(),
    );
    agent.enable_sessions(home.path()).unwrap();
    let store =
        crate::sessions::Store::new(home.path().to_path_buf(), std::path::Path::new(&workspace))
            .unwrap();
    let target = store.fixture_journal(&id);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::copy(&journal, target).unwrap();
    require_settled_journal(&store, &id).unwrap_or_else(|error| panic!("{error}"));
    // Fixture subscription accounts let a journal from any provider resume;
    // requests are never made.
    let config = super::tests::config(&home);
    let config_home = config.store.path().parent().unwrap().to_path_buf();
    crate::auth::openai::Store::fixture_account(config_home.clone(), "isolated-token").unwrap();
    crate::auth::anthropic::Store::fixture_account(config_home.clone(), "isolated-token").unwrap();
    for services in config.settings.configured_services(&config_home) {
        agent.register_backend(services);
    }
    let mut app = App::new(agent, config, None);
    let started = Instant::now();
    resume_for_measurement(&mut app, &id).unwrap_or_else(|error| panic!("{error}"));
    println!("RESUME,{:.3}", started.elapsed().as_secs_f64() * 1e3);
    let items = app.state.items.len();
    scroll_samples(&mut app, items);
}

/// Wheel steps through the application and their parts, at two sizes.
fn scroll_samples(app: &mut App, size: usize) {
    println!("ITEMS,{size},{}", app.state.items.len());
    for (width, height) in [(80, 24), (160, 50)] {
        app.state.width = width;
        app.state.height = height;
        app.renderer = Display::new((width, height));
        let started = Instant::now();
        drain(&mut app.renderer, &app.state);
        println!(
            "LAYOUT,{size},{width}x{height},{:.3}",
            started.elapsed().as_secs_f64() * 1e3
        );
        // One wheel notch through the application, as the event loop runs it.
        let mut decoder = super::keys::Decoder::default();
        samples(&format!("wheel_loop_{width}x{height}"), size, || {
            app.terminal_event(super::terminal::Input::Scroll(-3), &mut decoder)
                .unwrap();
            app.capture_input();
            app.persist_input(false);
            drain(&mut app.renderer, &app.state)
        });
        samples(&format!("scroll_layout_{width}x{height}"), size, || {
            app.renderer.scroll(Scroll::Rows(-3));
            drain(&mut app.renderer, &app.state)
        });
        samples(&format!("capture_input_{width}x{height}"), size, || {
            app.capture_input();
            0
        });
        samples(&format!("poll_settled_{width}x{height}"), size, || {
            app.poll().unwrap();
            0
        });
        app.state.toggle_tools();
        drain(&mut app.renderer, &app.state);
        samples(&format!("scroll_expanded_{width}x{height}"), size, || {
            app.renderer.scroll(Scroll::Rows(-3));
            drain(&mut app.renderer, &app.state)
        });
        app.state.toggle_tools();
    }
}

#[path = "measurement_tests/own_history_tests.rs"]
mod own_history_tests;
